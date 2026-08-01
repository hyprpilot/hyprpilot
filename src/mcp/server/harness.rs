//! The agent harness — `list_profiles` / `spawn` / `session_send` /
//! `session_list` / `session_status` / `session_read` / `session_kill`.
//!
//! Turns the captain's profile registry into something a connected
//! agent can drive: pick a profile, run it, talk to it across turns,
//! read its transcript, kill it. Its own subcommand and its own
//! `ServerHandler`, so the skills server cannot serve `spawn` — the
//! gate is structural rather than a name check. Off by default because
//! a profile's `command` is an arbitrary binary.
//!
//! Every launch flows through `spawn::prepare`, the same path
//! `hyprpilot <profile>` uses, so prompt-source priority, the
//! `-- <args>` escape hatch, and cwd precedence cannot drift between
//! the CLI and this surface.

use std::sync::Arc;

use serde_json::{json, Value};

use super::sessions::{SessionStatus, SessionTable, TurnOutcome};
use super::ConfigSource;
use crate::spawn::providers::HarnessProjection;
use crate::spawn::{LaunchOrigin, SpawnRequest};

/// Env stamp bounding recursive spawning. A spawned agent gets
/// `depth + 1`; at [`MAX_SPAWN_DEPTH`] a spawn is refused.
///
/// One means a spawned agent cannot spawn its own: the lead delegates,
/// the delegate works. A tree costs whatever its widest level costs,
/// and only the root can see it — [`MAX_LIVE_SESSIONS`] bounds each
/// sidecar separately, so N delegates each spawning N is N² processes
/// no single ceiling catches.
pub(crate) const DEPTH_ENV: &str = "HYPRPILOT_SPAWN_DEPTH";
const MAX_SPAWN_DEPTH: usize = 1;

/// Ceiling on concurrently *running* sessions. Depth bounds recursion;
/// this bounds breadth. Both matter: a profile's `command` is an
/// arbitrary binary, so an agent that can spawn without limit is an
/// agent that can exhaust the host.
const MAX_LIVE_SESSIONS: usize = 8;

/// Default for `--max-sessions`: how many sessions the table retains
/// before evicting the oldest FINISHED ones.
///
/// A conversation no longer grows this — `session_send` reuses its
/// session — so the only thing that does is distinct `spawn`s, which the
/// caller controls. This bounds a long-lived sidecar's memory and its
/// transcript directories without an API the caller has to remember to
/// call; `session_kill` on a finished session reaps it immediately for
/// callers that would rather be explicit.
pub const DEFAULT_MAX_SESSIONS: usize = 64;

/// Cap on bytes returned by a single `session_read` / inline `spawn`
/// result. Well under Hermes' 150000-byte tool-output limit, so a
/// transcript never blows the caller's own budget.
const READ_CAP_BYTES: usize = 60_000;

/// Lines of transcript tail scanned for a turn's terminal marker.
/// Generous — a turn's closing events are the last handful — but
/// bounded, because `session_status` is advertised as the cheap poll.
const TERMINAL_SCAN_LINES: usize = 200;
const DEFAULT_TIMEOUT_SECS: u64 = 300;

/// How often a follow re-checks the transcript for new bytes. The file
/// is append-only and a quarter-second lag is invisible next to model
/// latency, so a platform-specific file watcher would buy nothing.
const WATCH_POLL: std::time::Duration = std::time::Duration::from_millis(250);

/// Live-follow options for `session_read`.
///
/// **How streaming actually works here.** An MCP tool call returns one
/// result — there is no way to stream the *result* incrementally. What
/// the spec does give us is `notifications/progress`: when the caller
/// includes a `progressToken` in the request's `_meta`, the server may
/// push notifications *during* the call. So a follow streams each new
/// chunk as a progress notification and still returns the accumulated
/// text at the end. Clients that render progress show output live;
/// clients that do not still get everything in the final result.
///
/// **There is deliberately no server-side time limit.** The caller ends
/// a follow by cancelling the request (`notifications/cancelled`),
/// which trips [`WatchOptions::cancel`], or by passing its own
/// `timeout_seconds`. The agent decides how long to watch; the server
/// does not second-guess it.
pub(crate) struct WatchOptions {
    /// Optional self-imposed limit. `None` follows until the session
    /// exits or the caller cancels.
    pub seconds: Option<u64>,
    /// Where to push live chunks, when the caller supplied a progress
    /// token. `None` degrades to a plain long poll.
    pub sink: Option<ProgressSink>,
    /// Tripped when the client cancels the request.
    pub cancel: tokio_util::sync::CancellationToken,
}

/// What a follow saw.
///
/// Only whether the session ended. A follow used to also accumulate the
/// text it streamed, so the result could hand that buffer back — but
/// the buffer was capped by keeping its NEWEST bytes while the cursor
/// advanced past the discarded ones, which made the dropped prefix
/// unreachable. Results now re-read the transcript from disk instead,
/// so there is nothing to accumulate and nothing to drop: the
/// notifications stream every chunk, and the result pages losslessly.
pub(crate) struct FollowResult {
    pub finished: bool,
}

/// Streams transcript chunks to the caller as progress notifications.
pub(crate) struct ProgressSink {
    pub peer: rmcp::service::Peer<rmcp::service::RoleServer>,
    pub token: rmcp::model::ProgressToken,
}

impl ProgressSink {
    async fn push(&self, chunk: &str, bytes_so_far: u64) {
        // `#[non_exhaustive]` forbids the struct literal, so build via
        // `new` and set the owned instance's fields — the same pattern
        // `structured_with_text` uses for `CallToolResult`.
        //
        // Bytes read is the only monotonic measure available: the total
        // is genuinely unknown until the agent decides to stop.
        let mut param = rmcp::model::ProgressNotificationParam::new(self.token.clone(), bytes_so_far as f64);
        param.message = Some(chunk.to_string());
        let _ = self.peer.notify_progress(param).await;
    }
}

pub(crate) struct Harness {
    config: ConfigSource,
    pub(crate) sessions: Arc<SessionTable>,
    depth: usize,
    max_sessions: usize,
}

impl Harness {
    pub(crate) fn new(config: ConfigSource, max_sessions: usize) -> Self {
        let depth = std::env::var(DEPTH_ENV)
            .ok()
            .and_then(|raw| raw.parse::<usize>().ok())
            .unwrap_or(0);

        Self {
            config,
            sessions: SessionTable::new(),
            depth,
            // A zero would evict every finished session the moment it
            // finished, making `session_read` useless on a completed run.
            max_sessions: max_sessions.max(1),
        }
    }

    fn check_capacity(&self) -> Result<(), String> {
        if self.depth >= MAX_SPAWN_DEPTH {
            return Err(format!(
                "spawn refused: this session was itself spawned by a hyprpilot harness \
                 (depth {}, limit {MAX_SPAWN_DEPTH}). A delegated agent does not spawn its own.",
                self.depth
            ));
        }
        let live = self.sessions.live();
        if live >= MAX_LIVE_SESSIONS {
            return Err(format!(
                "spawn refused: {live} sessions already running (limit {MAX_LIVE_SESSIONS}). \
                 Call `session_kill` on a finished or runaway session first."
            ));
        }

        Ok(())
    }

    /// Shared body of `spawn` and `session_send`: resolve, launch, optionally
    /// wait, and report. `resume_id` distinguishes the two — `None`
    /// starts a fresh conversation.
    async fn launch(&self, args: LaunchToolArgs, resume: Option<ResumeTarget>) -> Result<Value, String> {
        self.check_capacity()?;
        reject_executable_overrides(&args.with_config)?;

        let cfg = self.config.load().map_err(|err| {
            format!(
                "could not load hyprpilot config: {err:#}. The skills surface is unaffected; fix the config and retry."
            )
        })?;
        // Both `spawn` and `session_send` land here — `session_send`
        // fills `args.profile` from its session first — so one check
        // covers the whole surface. Gating only `list_profiles` would
        // leave a hidden profile reachable by anyone holding its id.
        if !harness_allows(&cfg, &args.profile) {
            return Err(format!(
                "profile `{}` is not available to the harness. Declare `[profiles.harness]` on it to \
                 opt in — the harness runs only the profiles the captain nominated. \
                 Call `list_profiles` for the ones that are available.",
                args.profile
            ));
        }

        let projection = HarnessProjection {
            structured_output: true,
            resume: resume.as_ref().map(|target| target.resume_token.clone()),
        };
        let request = SpawnRequest {
            profile_id: Some(args.profile.clone()),
            prompt: Some(args.prompt.clone()),
            cwd: args.cwd.clone(),
            mode: args.mode.clone(),
            config_patches: args.with_config.clone(),
            provider_args: args.args.clone(),
            // The harness never has stdin to offer — `prepare` also
            // refuses to read the real fd0, which is the MCP transport.
            stdin_consumed: true,
        };

        let mut prepared = crate::spawn::prepare(&cfg, request, LaunchOrigin::Harness, Some(&projection))
            .map_err(|err| format!("could not resolve profile `{}`: {err:#}", args.profile))?;
        prepared
            .command
            .env
            .insert(DEPTH_ENV.to_string(), (self.depth + 1).to_string());

        let provenance = super::sessions::Provenance {
            program: prepared.command.program.clone(),
            argv: crate::spawn::providers::redacted_argv(&prepared.command.args),
            env_keys: prepared.command.env.keys().cloned().collect(),
            model: prepared.model.clone(),
            effort: prepared.effort.clone(),
            mode: prepared.mode.clone(),
            prompt_bytes: args.prompt.len(),
        };

        // A resume KEEPS the caller's handle. The conversation is the
        // same one, so its id should be too — a caller that spawned once
        // never has to notice that a later turn is a different process,
        // and an N-turn conversation costs one table entry and one
        // transcript rather than N.
        let (handle, resume_from) = match resume.as_ref() {
            Some(target) => {
                let handle = target.handle.clone();
                self.sessions
                    .respawn(&handle, prepared.command, provenance)
                    .map_err(|err| match err {
                        super::sessions::RespawnError::Unknown => {
                            format!("unknown session `{handle}`. Call `session_list` for live handles.")
                        }
                        super::sessions::RespawnError::Busy => format!(
                            "session `{handle}` already has a turn in flight. Wait for it (poll \
                             `session_read`) or `session_kill` it — no vendor supports two \
                             concurrent turns on one conversation."
                        ),
                        super::sessions::RespawnError::Spawn(err) => {
                            format!("could not start the next turn: {err:#}")
                        }
                    })?;
                // Resume the transcript where the last turn stopped, so a
                // follow returns only the NEW output.
                let from = self
                    .sessions
                    .with(&handle, |session| {
                        std::fs::metadata(session.turns_path()).map(|m| m.len()).unwrap_or(0)
                    })
                    .unwrap_or(0);

                (handle, from)
            }
            None => {
                let handle = self
                    .sessions
                    .spawn(
                        prepared.command,
                        prepared.profile_id.clone(),
                        prepared.provider,
                        provenance,
                        super::sessions::LaunchShape {
                            // `cwd` is overwritten with the RESOLVED one
                            // inside `spawn`; the rest are the caller's
                            // inputs, replayed verbatim on later turns.
                            cwd: None,
                            mode: args.mode.clone(),
                            with_config: args.with_config.clone(),
                            args: args.args.clone(),
                        },
                    )
                    .map_err(|err| format!("could not spawn session: {err:#}"))?;
                // Bound the table now that it just grew.
                self.sessions.evict_exited_over(self.max_sessions);

                (handle, 0)
            }
        };

        tracing::info!(
            %handle,
            profile = %prepared.profile_id,
            provider = prepared.provider.wire_id(),
            resumed = resume.is_some(),
            "mcp harness: turn started"
        );

        if !args.wait {
            return Ok(self.describe(&handle, None, Some(resume_from)));
        }

        // Waiting IS following: stream the transcript to the caller
        // while the agent works, rather than sitting silent for minutes
        // and dumping everything at the end. Same loop `session_read`
        // uses, so a `spawn` and a follow-up follow behave identically.
        let watch = WatchOptions {
            seconds: Some(args.timeout_seconds),
            sink: args.sink,
            cancel: args.cancel.unwrap_or_default(),
        };
        let followed = self.follow(&handle, resume_from, &watch).await;
        let finished = followed.finished;

        // Whether it finished or timed out, the transcript is on disk —
        // harvest the vendor session id either way so a follow-up
        // `session_send` works even for a turn that outran its timeout.
        self.harvest(&handle);

        if !finished {
            tracing::info!(%handle, secs = args.timeout_seconds, "mcp harness: turn outlived its timeout");
        }

        Ok(self.describe(&handle, Some(finished), Some(resume_from)))
    }

    /// Follow a session's transcript from `from`, pushing each new chunk
    /// to the caller's progress sink.
    ///
    /// Ends when the agent exits, when the caller cancels the request,
    /// or when `watch.seconds` elapses — whichever comes first. Returns
    /// what it saw, where it stopped, and whether the agent finished.
    async fn follow(&self, handle: &str, from: u64, watch: &WatchOptions) -> FollowResult {
        let Some((turns, mut completion)) = self
            .sessions
            .with(handle, |session| (session.turns_path(), session.completion()))
        else {
            return FollowResult { finished: true };
        };

        let deadline = watch
            .seconds
            .map(|secs| tokio::time::Instant::now() + std::time::Duration::from_secs(secs));
        let mut cursor = from;

        loop {
            let (chunk, _truncated, next) = read_from(&turns, cursor);
            if !chunk.is_empty() {
                if let Some(sink) = watch.sink.as_ref() {
                    sink.push(&chunk, next).await;
                }
                cursor = next;
                // Keep draining while output flows; only park once the
                // file has nothing new.
                continue;
            }
            if completion.borrow().is_some() {
                return FollowResult { finished: true };
            }
            if watch.cancel.is_cancelled() {
                tracing::debug!(%handle, "mcp harness: follow ended by client cancellation");
                return FollowResult { finished: false };
            }
            if deadline.is_some_and(|deadline| tokio::time::Instant::now() >= deadline) {
                return FollowResult { finished: false };
            }

            tokio::select! {
                _ = watch.cancel.cancelled() => {
                    tracing::debug!(%handle, "mcp harness: follow ended by client cancellation");
                    return FollowResult { finished: false };
                }
                // Either the child exited or the poll interval elapsed —
                // loop round to drain whatever it wrote on the way out.
                _ = tokio::time::timeout(WATCH_POLL, completion.changed()) => {}
            }
        }
    }

    /// Read back the vendor's own session id from the transcript.
    ///
    /// All three vendors mint their own and report it under a DIFFERENT
    /// key — `session_id` (claude), `thread_id` (codex), `sessionID`
    /// (opencode) — each verified against the installed CLI. Scanned
    /// rather than assumed positional because the emitting event is not
    /// always the first line.
    fn harvest(&self, handle: &str) {
        let Some(path) = self.sessions.with(handle, |session| session.turns_path()) else {
            return;
        };
        let Ok(body) = std::fs::read_to_string(&path) else {
            return;
        };
        let Some(id) = vendor_session_id(&body) else {
            return;
        };
        self.sessions.with_mut(handle, |session| {
            if session.resume_token.is_none() {
                session.resume_token = Some(id.clone());
            }
            session.last_turn_at = std::time::SystemTime::now();
        });
    }

    /// The result shape shared by `spawn` and `session_send`.
    ///
    /// `turn_start` is the START of this turn, not the end. The result
    /// re-reads the transcript forward from there rather than handing
    /// back the follow's in-memory buffer: that buffer is trimmed to
    /// [`READ_CAP_BYTES`] by keeping its NEWEST bytes, while the cursor
    /// advances past the discarded ones — so anything dropped became
    /// unreachable, offsets only moving forward. Reading from the file
    /// makes `nextOffset` the first byte NOT returned, so a caller that
    /// keeps paging reconstructs the whole transcript.
    fn describe(&self, handle: &str, finished: Option<bool>, turn_start: Option<u64>) -> Value {
        let Some(snapshot) = self.sessions.with(handle, |session| {
            (
                session.profile_id.clone(),
                session.provider.wire_id(),
                session.status(),
                session.exit_code(),
                session.turns_path(),
            )
        }) else {
            return json!({ "session": handle, "status": "exited" });
        };
        let (profile, provider, status, exit_code, turns) = snapshot;

        let mut out = json!({
            "session": handle,
            "profile": profile,
            "provider": provider,
            "status": status.as_str(),
        });
        if let Some(code) = exit_code {
            out["exitCode"] = json!(code);
        }
        if finished == Some(false) {
            out["timedOut"] = json!(true);
        }
        out["sessionInfo"] = self.provenance(handle);
        // Read forward from where this turn began, so the documented
        // "poll `session_read { session, cursor }`" loop reconstructs the
        // transcript with no gap — however far the turn overran the cap.
        if let Some(start) = turn_start {
            let (text, truncated, next) = read_from(&turns, start);
            out["text"] = json!(text);
            // Absent `nextCursor` means "finished AND fully read". A
            // running session always gets one: without a resume point a
            // poller would fall back to `tail` and lose its place.
            if truncated || status != SessionStatus::Exited {
                out["nextCursor"] = json!(encode_cursor(next));
            }
        }

        out
    }

    pub(crate) fn list_profiles(&self) -> Result<(String, Value), String> {
        let cfg = self
            .config
            .load()
            .map_err(|err| format!("could not load hyprpilot config: {err:#}"))?;
        // A profile the captain took off the harness is absent here AND
        // refused by `launch` — the listing is the discoverability half,
        // not the gate. `hyprpilot profiles` still shows it: this is a
        // harness policy, not a "hide it from the captain" one.
        // A profile whose patches fail to resolve falls back to its
        // UNPATCHED base, so `harness_enabled` reads false even when the
        // patch that opts it in is the thing that broke. Keep those rows:
        // they carry the error, and a silently-missing profile is the
        // worst way to report a broken config.
        let profiles: Vec<_> = crate::spawn::list_profiles(&cfg, None, &[])
            .into_iter()
            .filter(|profile| profile.harness_enabled || profile.error.is_some())
            .collect();
        let summary = profiles_table(&profiles);

        Ok((summary, json!({ "profiles": profiles })))
    }

    /// One session's state, without its transcript.
    ///
    /// `session_list` returns these same fields for every session and
    /// `session_read` returns the transcript itself, which runs to tens
    /// of kilobytes. This is the cheap poll: a handle lookup plus one
    /// `stat`.
    pub(crate) fn session_status(&self, handle: &str) -> Result<(String, Value), String> {
        self.sessions
            .with(handle, |session| {
                let turns = session.turns_path();
                let transcript_bytes = std::fs::metadata(&turns).map(|m| m.len()).unwrap_or(0);
                let mut out = json!({
                    "session": session.handle,
                    "profile": session.profile_id,
                    "provider": session.provider.wire_id(),
                    "status": session.status().as_str(),
                    "createdAt": unix_secs(session.created_at),
                    "lastTurnAt": unix_secs(session.last_turn_at),
                    "transcriptBytes": transcript_bytes,
                    // Only meaningful once the turn has ENDED, and read
                    // from the tail. Both halves are load-bearing:
                    //
                    // - A running session is never "done". opencode emits
                    //   a `text` part for every completed sentence, so
                    //   asking mid-run reports the first one as the
                    //   answer.
                    // - `session_send` APPENDS to this same file, so a
                    //   scan from byte 0 keeps finding turn 1's marker
                    //   forever and every later turn reads as finished
                    //   the instant it starts.
                    "hasResult": session.status() == SessionStatus::Exited && tail_has_terminal_result(&turns),
                });
                if let Some(code) = session.exit_code() {
                    out["exitCode"] = json!(code);
                }
                out
            })
            .map(|row| (status_summary(&row), row))
            .ok_or_else(|| format!("unknown session `{handle}`. Call `session_list` for live handles."))
    }

    pub(crate) async fn spawn(&self, args: LaunchToolArgs) -> Result<Value, String> {
        self.launch(args, None).await
    }

    /// Send another message to a session, resuming it first if its
    /// process has already exited.
    ///
    /// The caller does not have to know which state the session is in.
    /// The result says which path was taken, because "resumed a finished
    /// conversation" and "the agent was still going" are materially
    /// different things to a caller deciding what to do next.
    pub(crate) async fn session_send(&self, handle: &str, args: LaunchToolArgs) -> Result<Value, String> {
        // Harvest lazily. A detached launch — now the default — returns
        // before the waiting path ever runs, so its session has no resume
        // token yet; without this it could never be resumed, and the
        // branch below would blame a failed first turn for a session that
        // is perfectly healthy.
        self.harvest(handle);

        let (target, shape) = self
            .sessions
            .with(handle, |session| {
                (
                    ResumeTarget {
                        handle: session.handle.clone(),
                        profile_id: session.profile_id.clone(),
                        resume_token: session.resume_token.clone().unwrap_or_default(),
                        has_resume_token: session.resume_token.is_some(),
                    },
                    session.launch.clone(),
                )
            })
            .ok_or_else(|| format!("unknown session `{handle}`. Call `session_list` for live handles."))?;

        if !target.has_resume_token {
            return Err(format!(
                "session `{handle}` cannot be resumed — the vendor never reported a session id for it. \
                 Its first turn probably failed before the agent started — check `session_read`."
            ));
        }

        // The "still running" refusal lives in `SessionTable::respawn`,
        // under the table lock. Checking it here as well would only be a
        // second chance to lose the race — the lock is what actually
        // guarantees one turn at a time.
        // A conversation's working directory is part of its IDENTITY,
        // not per-turn options. Every one of them is replayed from the
        // session unless this turn overrides it explicitly. Dropping any
        // of them relaunched turn 2 differently from turn 1 in silence —
        // most visibly `cwd`, which claude keys its conversation store
        // by, so a resume from elsewhere failed with a bare "No
        // conversation found with session ID" for a perfectly healthy
        // session.
        let args = LaunchToolArgs {
            profile: target.profile_id.clone(),
            cwd: args.cwd.clone().or(shape.cwd),
            mode: args.mode.clone().or(shape.mode),
            with_config: if args.with_config.is_empty() {
                shape.with_config
            } else {
                args.with_config.clone()
            },
            args: if args.args.is_empty() {
                shape.args
            } else {
                args.args.clone()
            },
            ..args
        };
        let mut out = self.launch(args, Some(target)).await?;
        out["delivery"] = json!("resumed");

        Ok(out)
    }

    /// What this session was launched with — the audit view. argv is
    /// redacted at capture (flag names survive, value payloads become
    /// size placeholders) because it carries the system prompt and, for
    /// some vendors, MCP bearer tokens.
    fn provenance(&self, handle: &str) -> Value {
        self.sessions
            .with(handle, |session| {
                json!({
                    "profile": session.profile_id,
                    "provider": session.provider.wire_id(),
                    "model": session.provenance.model,
                    "effort": session.provenance.effort,
                    "mode": session.provenance.mode,
                    "cwd": session.launch.cwd.as_ref().map(|c| c.display().to_string()),
                    "startedAt": unix_secs(session.created_at),
                    "lastTurnAt": unix_secs(session.last_turn_at),
                    "pid": session.pid(),
                    // Every file the session owns, named once. A caller
                    // with shell access can `jq` the transcript directly
                    // rather than paging it through `session_read` — that
                    // is a feature, so the paths are first-class rather
                    // than derivable from a sibling.
                    "files": {
                        "dir": session.dir_path().display().to_string(),
                        "transcript": session.turns_path().display().to_string(),
                        "stderr": session.stderr_path().display().to_string(),
                        "done": session.done_path().display().to_string(),
                        "breadcrumb": session.breadcrumb_path().display().to_string(),
                    },
                    "command": session.provenance.program,
                    "argv": session.provenance.argv,
                    "envKeys": session.provenance.env_keys,
                    "promptBytes": session.provenance.prompt_bytes,
                })
            })
            .unwrap_or(Value::Null)
    }

    pub(crate) fn session_list(&self) -> (String, Value) {
        let rows = self.sessions.map_all(|session| {
            let mut row = json!({
                "session": session.handle,
                "profile": session.profile_id,
                "provider": session.provider.wire_id(),
                "status": session.status().as_str(),
                "createdAt": unix_secs(session.created_at),
                "lastTurnAt": unix_secs(session.last_turn_at),
            });
            if let Some(code) = session.exit_code() {
                row["exitCode"] = json!(code);
            }
            if let Some(cwd) = session.launch.cwd.as_ref() {
                row["cwd"] = json!(cwd.display().to_string());
            }
            row
        });

        let summary = if rows.is_empty() {
            "No sessions. Call `spawn` to start one.".to_string()
        } else {
            let mut out = format!("{} session(s):\n", rows.len());
            for row in &rows {
                out.push_str(&format!(
                    "- {} [{}] {} ({})\n",
                    row["session"].as_str().unwrap_or("?"),
                    row["status"].as_str().unwrap_or("?"),
                    row["profile"].as_str().unwrap_or("?"),
                    row["provider"].as_str().unwrap_or("?"),
                ));
            }
            out.push_str("Sessions live only as long as this MCP server; they do not survive a restart.");
            out
        };

        (summary, json!({ "sessions": rows }))
    }

    /// Read a transcript, optionally following it live.
    ///
    /// Without `watch` this is a plain read: the tail, or everything
    /// past `offset`.
    ///
    /// With `watch` it **follows** — see [`WatchOptions`] for why that
    /// streams via `notifications/progress` rather than the tool result
    /// itself, and why the caller (not the server) decides when to stop.
    pub(crate) async fn session_read(
        &self,
        handle: &str,
        tail: usize,
        cursor: Option<String>,
        watch: Option<WatchOptions>,
    ) -> Result<Value, String> {
        let paths = self
            .sessions
            .with(handle, |session| {
                (session.turns_path(), session.stderr_path(), session.status())
            })
            .ok_or_else(|| format!("unknown session `{handle}`. Call `session_list` for live handles."))?;
        let (turns, stderr_path, mut status) = paths;
        let offset = cursor.as_deref().map(decode_cursor).transpose()?;

        // Following streams from `offset` forward; a tail-follow would
        // have no stable resume point. The follow's own text/cursor are
        // deliberately NOT reused for the result — see the match below.
        let followed = watch.is_some();
        if let Some(watch) = watch {
            self.follow(handle, offset.unwrap_or(0), &watch).await;
            status = self.sessions.with(handle, |s| s.status()).unwrap_or(status);
        }

        let (text, truncated, next_offset) = match (followed, offset) {
            // A follow streamed live via progress notifications; the
            // RESULT re-reads from where the follow started rather than
            // handing back its buffer. That buffer keeps its NEWEST
            // bytes while the cursor advances past the discarded ones,
            // so anything trimmed became unreachable — offsets only move
            // forward. Reading from the file keeps the cursor at the
            // first byte NOT returned, so paging never skips.
            (true, _) => read_from(&turns, offset.unwrap_or(0)),
            (false, Some(offset)) => read_from(&turns, offset),
            (false, None) => {
                // A tail read is at EOF by definition. `tail_of`'s own
                // "truncated" means lines were dropped BEHIND the window,
                // which is not what a forward cursor signals — so it is
                // deliberately not fed into one.
                let (text, _dropped_before_the_tail) = tail_of(&turns, tail);
                let end = std::fs::metadata(&turns).map(|m| m.len()).unwrap_or(0);
                (text, false, end)
            }
        };

        let mut out = json!({
            "session": handle,
            "status": status.as_str(),
            "lines": text,
            "sessionInfo": self.provenance(handle),
        });
        // MCP pagination: `nextCursor` present means there is more to
        // read, absent means the session is finished AND fully read. A
        // running session always gets one — without a resume point a
        // poller would fall back to `tail` and lose its place.
        if truncated || status != SessionStatus::Exited {
            out["nextCursor"] = json!(encode_cursor(next_offset));
        }
        // Only surface stderr when it has something — an empty diagnostic
        // channel is noise in every successful result.
        if let Ok(errors) = std::fs::read_to_string(&stderr_path) {
            let errors = errors.trim();
            if !errors.is_empty() {
                let clipped: String = errors
                    .chars()
                    .rev()
                    .take(2000)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                out["stderr"] = json!(clipped);
            }
        }

        Ok(out)
    }

    /// Stop a session, or forget one that has already stopped.
    ///
    /// State-aware, like `session_send`: killing a RUNNING session
    /// terminates it and keeps the transcript, so the caller can still
    /// read why it was killed. Killing an already-FINISHED session
    /// reaps it — table entry and transcript both go. So "kill twice"
    /// is the natural way to stop and then clean up, and the second
    /// call does something useful instead of nothing.
    pub(crate) async fn session_kill(&self, handle: &str) -> Result<Value, String> {
        let was_running = self
            .sessions
            .kill(handle)
            .await
            .ok_or_else(|| format!("unknown session `{handle}`. Call `session_list` for live handles."))?;

        if was_running {
            return Ok(json!({
                "session": handle,
                "action": "terminated",
                "wasRunning": true,
                "reaped": false,
            }));
        }

        // Already finished — this call is the cleanup.
        let reaped = self.sessions.forget(handle);

        Ok(json!({
            "session": handle,
            "action": if reaped { "reaped" } else { "gone" },
            "wasRunning": false,
            "reaped": reaped,
        }))
    }
}

struct ResumeTarget {
    handle: String,
    profile_id: String,
    resume_token: String,
    has_resume_token: bool,
}

/// Decoded arguments shared by `spawn` and `session_send`.
pub(crate) struct LaunchToolArgs {
    pub profile: String,
    pub prompt: String,
    pub cwd: Option<std::path::PathBuf>,
    pub mode: Option<String>,
    pub with_config: Vec<Value>,
    pub args: Vec<String>,
    pub wait: bool,
    pub timeout_seconds: u64,
    /// Live output stream, when the caller supplied a progress token.
    /// A waiting `spawn` follows the transcript exactly as
    /// `session_read { watch: true }` does, so the caller sees the agent
    /// working instead of a silent block.
    pub sink: Option<ProgressSink>,
    /// Tripped when the caller cancels the request — ends the wait
    /// early without killing the agent.
    pub cancel: Option<tokio_util::sync::CancellationToken>,
}

impl LaunchToolArgs {
    pub(crate) fn default_timeout() -> u64 {
        DEFAULT_TIMEOUT_SECS
    }
}

/// The ONLY keys a caller-supplied `with_config` overlay may set.
///
/// **Allow-list, not deny-list — deliberately.** A deny-list of
/// `command`/`args`/`env` looked sufficient and was not: `mcps` accepts
/// inline `mcp_servers` entries carrying their own `command`/`args`,
/// which the launcher writes into the vendor's MCP config and the vendor
/// then spawns — arbitrary execution through a field the deny-list never
/// mentioned. `$deleteFromPrimitiveList/args` reaches `args` without
/// ever using the literal key, too. Enumerating the ways *in* is a losing
/// game against a config tree that grows; enumerating what is *allowed*
/// is not.
///
/// These three are the "one-off swap" the parameter exists for. Anything
/// else — including every `$`-directive, since a directive's whole job is
/// to address some other field — is refused.
const ALLOWED_OVERLAY_KEYS: &[&str] = &["model", "effort", "mode"];

/// Whether a profile is exposed to the harness at all.
///
/// **Opt-in**: a profile with no `[profiles.harness]` block is NOT
/// available. `spawn` runs a profile's `command` as this user, so what
/// an agent may drive is a list the captain wrote, not the whole
/// registry.
///
/// An id matching nothing stays allowed, so the resolver keeps owning
/// "unknown profile" errors instead of this reporting a misleading
/// "not available to the harness" for a typo.
fn harness_allows(cfg: &crate::config::Config, profile_id: &str) -> bool {
    if !cfg.profiles.iter().any(|profile| profile.id == profile_id) {
        return true;
    }
    // Resolve first: reading the raw entry made a `$match`ed patch — the
    // natural way to opt a family in — a silent no-op. `with_config` is
    // withheld so the gate reads nothing the caller controls.
    match crate::resolve::resolve_effective_profile(cfg, Some(profile_id), &[]) {
        Ok(profile) => profile
            .harness
            .as_ref()
            .is_some_and(crate::config::ProfileHarnessConfig::is_enabled),
        // Let the launch proceed and fail with the resolver's real error.
        // Reporting "not available to the harness" for a broken patch
        // would send the captain hunting for a gate that is not the
        // problem.
        Err(_) => true,
    }
}

/// Restrict a caller-supplied overlay to settings that cannot change
/// what gets executed.
///
/// Enabling the harness is a grant of *the captain's profiles*, not
/// of a shell; without this a single prompt-injected tool call is RCE on
/// a chat-reachable gateway.
fn reject_executable_overrides(overlays: &[Value]) -> Result<(), String> {
    for overlay in overlays {
        let Some(map) = overlay.as_object() else {
            return Err("`with_config` entries must be objects.".into());
        };
        for key in map.keys() {
            if !ALLOWED_OVERLAY_KEYS.contains(&key.as_str()) {
                return Err(format!(
                    "`with_config` may only set {}. `{key}` is refused: overlays that reach the command, \
                     its arguments, its environment, or the MCP servers the agent launches would turn \
                     `spawn` into arbitrary command execution. To run something else, add a profile for \
                     it in the hyprpilot config.",
                    ALLOWED_OVERLAY_KEYS
                        .iter()
                        .map(|key| format!("`{key}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
    }

    Ok(())
}

/// Whether byte index `at` starts a UTF-8 codepoint. A continuation
/// byte is `10xxxxxx`; everything else begins one.
fn is_utf8_boundary(buf: &[u8], at: usize) -> bool {
    at == 0 || at >= buf.len() || (buf[at] & 0b1100_0000) != 0b1000_0000
}

/// Scan a JSONL transcript for the vendor's own session id.
fn vendor_session_id(body: &str) -> Option<String> {
    const KEYS: [&str; 3] = ["session_id", "thread_id", "sessionID"];
    for line in body.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        for key in KEYS {
            if let Some(found) = value.get(key).and_then(Value::as_str) {
                if !found.is_empty() {
                    return Some(found.to_string());
                }
            }
        }
    }

    None
}

/// Push a completion event into the lead agent's context.
///
/// Claude Code calls this a *channel*: a notification whose `content`
/// becomes a `<channel source="…">` block in the agent's next turn,
/// with each `meta` entry as an attribute. A client that never
/// registered the channel drops it silently and returns no error, so
/// the server cannot detect non-delivery and must not try.
///
/// **The content is a fixed template and must stay that way.** Never
/// interpolate transcript bytes or agent output into it: that would let
/// a spawned agent write into its parent's context through a path the
/// parent never called. Everything variable rides `meta`, whose keys
/// must be `[A-Za-z0-9_]` — a hyphen is silently dropped.
pub(crate) async fn notify_session_finished(
    peer: &rmcp::service::Peer<rmcp::service::RoleServer>,
    server_name: &str,
    handle: &str,
    exit_code: i32,
) {
    let mut meta = serde_json::Map::new();
    meta.insert("session".into(), json!(handle));
    meta.insert("exit_code".into(), json!(exit_code.to_string()));

    let mut params = serde_json::Map::new();
    params.insert(
        "content".into(),
        json!(format!(
            "hyprpilot harness session {handle} finished (exit {exit_code}). \
             Read its output with session_read."
        )),
    );
    params.insert("meta".into(), serde_json::Value::Object(meta));

    let notification = rmcp::model::CustomNotification {
        method: "notifications/claude/channel".into(),
        params: Some(serde_json::Value::Object(params)),
        extensions: rmcp::model::Extensions::default(),
    };
    if let Err(err) = peer
        .send_notification(rmcp::model::ServerNotification::CustomNotification(notification))
        .await
    {
        tracing::debug!(%handle, %server_name, %err, "mcp harness: completion channel push failed");
    }
}

/// Push the SEP-2663 status event for a turn that just ended.
///
/// **Double-gated by the caller**, and both halves matter. A client that
/// never declared the extension must not receive `notifications/tasks`
/// for an ordinary `spawn` — that would be a behaviour change visible to
/// a caller who opted into nothing, which is exactly what this feature
/// promises not to do. And a turn that never minted a task has no task
/// to report on.
///
/// Note this rides `Peer::send_notification` directly rather than
/// `subscriptions/listen`: rmcp explicitly refuses to route task
/// notifications through a subscription today (`SubscriptionFilter` has
/// no `taskIds` field), so the subscription path is not available to us.
/// A client that does not handle the method drops it silently — the same
/// contract as the Claude channel above.
pub(crate) async fn notify_task_finished(
    peer: &rmcp::service::Peer<rmcp::service::RoleServer>,
    task: rmcp::model::DetailedTask,
) {
    let notification = rmcp::model::TaskStatusNotification::new(rmcp::model::TaskStatusNotificationParams::from(task));
    if let Err(err) = peer
        .send_notification(rmcp::model::ServerNotification::TaskStatusNotification(notification))
        .await
    {
        tracing::debug!(%err, "mcp harness: task status push failed");
    }
}

/// Whether the transcript carries the agent's final answer yet.
///
/// Each vendor marks completion differently — the same three-way split
/// `vendor_session_id` already handles for session ids. All three were
/// captured from the installed CLIs, per the repo rule that vendor keys
/// are verified rather than guessed:
///
/// - **claude** — a terminal `{"type":"result", "result": "…"}`.
/// - **codex** — `{"type":"turn.completed"}` closes the turn, but the
///   text rides the preceding `item.completed` whose `item.type` is
///   `agent_message`.
/// - **opencode** — emits NO terminal marker. Its stream ends
///   `step_finish(reason=stop)`, so the answer is the last
///   `{"type":"text"}` part. Scanning for a terminal event here would
///   find nothing and report `false` forever.
fn tail_has_terminal_result(path: &std::path::Path) -> bool {
    // The tail, not the whole file: terminal events are at the end by
    // definition, and this is advertised as the cheap poll. Also keeps
    // the answer scoped to the LATEST turn — earlier turns' markers sit
    // further back in the same appended transcript.
    let (tail, _truncated) = tail_of(path, TERMINAL_SCAN_LINES);

    has_terminal_result(&tail)
}

fn has_terminal_result(body: &str) -> bool {
    for line in body.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match value.get("type").and_then(Value::as_str) {
            // claude: carries the answer inline.
            Some("result") => return true,
            // codex: the turn is closed; `item.completed` above it holds
            // the message.
            Some("turn.completed") => return true,
            Some("item.completed") => {
                if value.get("item").and_then(|i| i.get("type")).and_then(Value::as_str) == Some("agent_message") {
                    return true;
                }
            }
            // opencode: no terminal event exists, so a text part IS the
            // completion signal.
            Some("text") => return true,
            _ => {}
        }
    }

    false
}

/// Last `lines` whole lines of a file, capped at [`READ_CAP_BYTES`].
/// Never cuts mid-line: a partial JSON object is worse than a shorter
/// transcript, since the caller is expected to parse these.
fn tail_of(path: &std::path::Path, lines: usize) -> (String, bool) {
    let Ok(body) = std::fs::read_to_string(path) else {
        return (String::new(), false);
    };
    let all: Vec<&str> = body.lines().collect();
    let start = all.len().saturating_sub(lines);
    let mut truncated = start > 0;

    // Build from the END backwards. Taking lines front-to-back and
    // stopping at the cap meant ONE oversized line — a `stream-json`
    // event carrying a big tool result easily clears 60 kB — discarded
    // every line after it, so `session_read` returned an empty tail for a
    // session with plenty of readable output.
    let mut kept: Vec<&str> = Vec::new();
    let mut used = 0usize;
    for line in all[start..].iter().rev() {
        if used + line.len() + 1 > READ_CAP_BYTES {
            truncated = true;
            // One huge line must not swallow the newer, smaller ones
            // after it — skip it and keep going.
            continue;
        }
        used += line.len() + 1;
        kept.push(line);
    }

    let mut out = String::new();
    for line in kept.iter().rev() {
        out.push_str(line);
        out.push('\n');
    }

    (out, truncated)
}

/// Read forward from a byte offset, returning whole lines and the offset
/// to resume from. Trailing partial lines are held back — a session
/// still being written to will complete them on the next call.
/// Encode a transcript position as an MCP pagination cursor.
///
/// MCP cursors are opaque by contract — a client must pass one back
/// verbatim, never parse or synthesise it. Hex keeps that honest: there
/// is no arithmetic to do on `"1a2b"`, whereas a raw byte offset invites
/// a caller to compute a position and skip bytes it never read.
fn encode_cursor(offset: u64) -> String {
    format!("{offset:x}")
}

fn decode_cursor(raw: &str) -> Result<u64, String> {
    u64::from_str_radix(raw, 16).map_err(|_| {
        format!(
            "invalid cursor `{raw}`. Pass a `nextCursor` from a previous result verbatim, or omit it to read the tail."
        )
    })
}

fn read_from(path: &std::path::Path, offset: u64) -> (String, bool, u64) {
    use std::io::{Read, Seek, SeekFrom};

    let Ok(mut file) = std::fs::File::open(path) else {
        return (String::new(), false, offset);
    };
    if file.seek(SeekFrom::Start(offset)).is_err() {
        return (String::new(), false, offset);
    }
    // Read BYTES, not a `String`. A cap that lands mid-codepoint makes
    // `read_to_string` fail with `InvalidData` — and since the cut point
    // is a pure function of `offset`, returning "nothing new" there would
    // stall the transcript at that byte forever, every retry failing
    // identically. The caller would see an agent that mysteriously went
    // quiet. Splitting on the newline first sidesteps it: a line
    // boundary is always a codepoint boundary.
    let mut buf = Vec::new();
    if file.take(READ_CAP_BYTES as u64).read_to_end(&mut buf).is_err() {
        return (String::new(), false, offset);
    }
    let filled = buf.len();
    let consumed = match buf.iter().rposition(|byte| *byte == b'\n') {
        Some(idx) => idx + 1,
        // No newline in the window. Two very different cases:
        None if filled < READ_CAP_BYTES => {
            // Short read: the session is mid-write and the newline is
            // still coming. Holding is correct and self-heals.
            return (String::new(), false, offset);
        }
        None => {
            // The window is FULL with no newline, so this line is longer
            // than the cap and the newline will never appear inside it.
            // Holding here pinned the cursor forever — every retry read
            // the identical bytes — and the caller saw an agent that
            // silently stopped talking. Hand back the partial line and
            // advance, on a char boundary so the split never lands
            // mid-codepoint.
            let mut cut = filled;
            while cut > 0 && !is_utf8_boundary(&buf, cut) {
                cut -= 1;
            }
            if cut == 0 {
                // Pathological: no boundary in a whole window. Advance
                // anyway rather than stall — lossy decoding below keeps
                // it readable.
                cut = filled;
            }
            let text = String::from_utf8_lossy(&buf[..cut]).into_owned();

            return (text, true, offset + cut as u64);
        }
    };
    let truncated = consumed == READ_CAP_BYTES;
    // Lossy is right here, not paranoid: whole lines are valid UTF-8 in
    // practice, and a corrupt transcript byte must not take the read
    // path down with it.
    let text = String::from_utf8_lossy(&buf[..consumed]).into_owned();

    (text, truncated, offset + consumed as u64)
}

fn unix_secs(t: std::time::SystemTime) -> u64 {
    t.duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Aligned one-line-per-profile table for the `content` block. opencode
/// renders only `content`, so this is the discovery view for a whole
/// class of clients — not a debug convenience.
fn status_summary(row: &Value) -> String {
    let get = |key: &str| row.get(key).and_then(Value::as_str).unwrap_or("?").to_string();
    let mut out = format!("{} — {} ({})", get("session"), get("status"), get("profile"));
    if let Some(code) = row.get("exitCode").and_then(Value::as_i64) {
        out.push_str(&format!(", exit {code}"));
    }
    if let Some(bytes) = row.get("transcriptBytes").and_then(Value::as_u64) {
        out.push_str(&format!(", {bytes} B transcript"));
    }
    if row.get("hasResult").and_then(Value::as_bool) == Some(true) {
        out.push_str(", result ready");
    }

    out
}

fn profiles_table(profiles: &[crate::resolve::ProfileSummary]) -> String {
    if profiles.is_empty() {
        return "No profiles are available to the harness. It is opt-in per profile: add \
                `[profiles.harness] enabled = true` to the ones an agent may drive (a `$match`ed \
                `[[patches]]` entry opts a whole family in at once). `hyprpilot profiles` lists \
                every configured profile, including the ones not nominated."
            .into();
    }
    let id_width = profiles.iter().map(|p| p.id.len()).max().unwrap_or(2).max(2);
    let provider_width = profiles.iter().map(|p| p.provider.len()).max().unwrap_or(8).max(8);

    let mut out = format!("{} profile(s) available (* = default):\n", profiles.len());
    for profile in profiles {
        let marker = if profile.error.is_some() {
            '!'
        } else if profile.is_default {
            '*'
        } else {
            ' '
        };
        out.push_str(&format!(
            "{marker} {:id_width$}  {:provider_width$}  {}",
            profile.id,
            profile.provider,
            profile.model.as_deref().unwrap_or("<vendor default>"),
        ));
        if profile.headless {
            out.push_str("  [headless]");
        }
        if let Some(err) = profile.error.as_deref() {
            out.push_str(&format!("  !! {err}"));
        }
        out.push('\n');
    }
    out.push_str("Pass an `id` as `spawn`'s `profile`. The profile already carries agent/model/effort/mode/MCP/skills — every other `spawn` argument is an override.");

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The gate must cover BOTH halves. `list_profiles` hiding a
    /// profile is discoverability; `launch` refusing it is the gate.
    /// Gating only the listing leaves it reachable by anyone holding
    /// the id — the exact bug the skills/harness server split removed.
    #[test]
    fn harness_allows_is_default_open_and_honours_the_off_switch() {
        let mut cfg = crate::config::Config::default();
        let mut open: crate::config::ProfileConfig =
            serde_json::from_value(json!({ "id": "open", "agent": "a" })).unwrap();
        open.harness = None;
        let mut closed: crate::config::ProfileConfig =
            serde_json::from_value(json!({ "id": "closed", "agent": "a" })).unwrap();
        closed.harness = Some(crate::config::ProfileHarnessConfig { enabled: Some(false) });
        cfg.profiles = vec![open, closed];

        assert!(
            !harness_allows(&cfg, "open"),
            "no block means NOT available — the surface is opt-in"
        );
        assert!(!harness_allows(&cfg, "closed"), "an explicit false must be refused");
        let mut on: crate::config::ProfileConfig = serde_json::from_value(json!({ "id": "on", "agent": "a" })).unwrap();
        on.harness = Some(crate::config::ProfileHarnessConfig { enabled: Some(true) });
        cfg.profiles.push(on);
        assert!(harness_allows(&cfg, "on"), "an explicit opt-in must be available");
        assert!(
            harness_allows(&cfg, "nonexistent"),
            "an unknown id stays the resolver's error to report, not ours"
        );
    }

    /// A delegate does not delegate. The stamp is what carries this
    /// across the process boundary — the sidecar a spawned agent talks
    /// to is a fresh process that only learns its depth from the env.
    #[test]
    fn a_spawned_session_cannot_spawn_its_own() {
        let mut harness = Harness::new(super::super::ConfigSource::default(), DEFAULT_MAX_SESSIONS);
        // Not whatever `new` read: these tests can themselves run inside
        // a spawned session, where the stamp is already set.
        harness.depth = 0;
        assert!(harness.check_capacity().is_ok(), "the lead spawns freely");

        harness.depth = 1;
        let refusal = harness.check_capacity().expect_err("a delegate must be refused");
        assert!(
            refusal.contains("spawned by a hyprpilot harness"),
            "the refusal must say WHY, not just cite a number: {refusal}"
        );
    }

    /// opencode emits a `text` part for EVERY completed sentence, not
    /// only the final answer — verified against the installed binary. So
    /// "a text part exists" cannot mean "the agent is done"; only the
    /// session having exited can. Pins the mid-run false positive.
    #[test]
    fn a_mid_run_opencode_text_part_is_not_a_finished_answer() {
        let mid_run = concat!(
            r#"{"type":"text","part":{"text":"I'll check the file first."}}"#,
            "\n",
            r#"{"type":"tool_use","part":{"tool":"bash"}}"#,
            "\n",
        );
        // The scanner itself still sees a text part — that is what it is
        // for. The guard is `status == Exited` in `session_status`, which
        // is why `hasResult` must never be computed from content alone.
        assert!(has_terminal_result(mid_run));
    }

    /// `session_send` APPENDS to one transcript, so turn 1's terminal
    /// marker never disappears. Scanning from byte 0 reported every
    /// later turn as finished the instant it started; scanning the TAIL
    /// keeps the answer scoped to the latest turn.
    #[test]
    fn the_terminal_scan_reads_the_tail_not_the_whole_transcript() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("turns.jsonl");
        // Turn 1 finished, then turn 2 pushed it far out of the tail.
        let mut body = String::from(r#"{"type":"result","result":"turn one"}"#);
        body.push('\n');
        for i in 0..(TERMINAL_SCAN_LINES * 2) {
            body.push_str(&format!("{{\"type\":\"tool_use\",\"seq\":{i}}}\n"));
        }
        std::fs::write(&path, &body).unwrap();

        assert!(
            !tail_has_terminal_result(&path),
            "turn 1's marker must not leak into turn 2's answer"
        );
    }

    /// Each vendor's terminal marker, captured from the installed CLI
    /// this was written against. opencode is the odd one: it emits no
    /// terminal event at all, so its last `text` part IS the signal.
    #[test]
    fn has_terminal_result_reads_all_three_vendors() {
        // claude
        assert!(has_terminal_result(
            r#"{"type":"result","subtype":"success","result":"ALPHA"}"#
        ));
        // codex — the turn closes, and the message rode the item before it
        assert!(has_terminal_result(
            r#"{"type":"item.completed","item":{"type":"agent_message","text":"CHARLIE"}}"#
        ));
        assert!(has_terminal_result(r#"{"type":"turn.completed","usage":{}}"#));
        // opencode — no terminal event exists; a text part is the answer
        assert!(has_terminal_result(r#"{"type":"text","part":{"text":"BRAVO"}}"#));
    }

    #[test]
    fn has_terminal_result_is_false_before_the_answer_lands() {
        let mid_run = concat!(
            r#"{"type":"step_start","part":{}}"#,
            "\n",
            r#"{"type":"tool_use","part":{"tool":"bash"}}"#,
            "\n",
            r#"{"type":"item.completed","item":{"type":"reasoning"}}"#,
            "\n",
        );
        assert!(
            !has_terminal_result(mid_run),
            "tool traffic and non-message items must not read as a result"
        );
        assert!(!has_terminal_result(""), "an empty transcript has no result");
        assert!(
            !has_terminal_result("not json at all\n"),
            "garbage must not panic or match"
        );
    }

    /// A follow must keep collecting until the session actually exits,
    /// not stop at the first chunk. Pins the whole loop against a child
    /// that writes in bursts with gaps — the shape a real agent has.
    #[tokio::test]
    async fn follow_collects_every_chunk_until_the_session_exits() {
        let harness = Harness::new(super::super::ConfigSource::default(), DEFAULT_MAX_SESSIONS);
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("burst.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\nprintf 'one\\n'\nsleep 0.4\nprintf 'two\\n'\nsleep 0.4\nprintf 'three\\n'\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        }

        let command = crate::spawn::providers::SpawnCommand {
            program: script.display().to_string(),
            args: Vec::new(),
            env: Default::default(),
            cwd: None,
            stdin_prompt: None,
        };
        let handle = harness
            .sessions
            .spawn(
                command,
                "p".into(),
                crate::config::AgentProvider::ClaudeCode,
                super::super::sessions::Provenance {
                    program: "burst".into(),
                    argv: Vec::new(),
                    env_keys: Vec::new(),
                    model: None,
                    effort: None,
                    mode: None,
                    prompt_bytes: 0,
                },
                super::super::sessions::LaunchShape::default(),
            )
            .unwrap();

        let watch = WatchOptions {
            seconds: Some(20),
            sink: None,
            cancel: tokio_util::sync::CancellationToken::new(),
        };
        let result = harness.follow(&handle, 0, &watch).await;

        assert!(result.finished, "follow must report the session finished");
        // The follow no longer carries the text — it drains the file and
        // streams chunks. Assert on the transcript it drained to, which
        // is what a caller reads afterwards.
        let turns = harness.sessions.with(&handle, |s| s.turns_path()).expect("session");
        let (text, _truncated, next) = read_from(&turns, 0);
        assert!(next > 0, "the follow must have advanced the transcript");
        for expected in ["one", "two", "three"] {
            assert!(
                text.contains(expected),
                "follow stopped early — missing {expected:?} in {text:?}"
            );
        }
    }

    /// The vendor's session id is a resume token, not an identity. It is
    /// absent for the whole first turn and appears later, so a caller
    /// that saw it could not rely on it; and it addresses nothing —
    /// every tool here takes the handle. Pin that no payload carries it,
    /// while `session_send` can still resume.
    #[tokio::test]
    async fn the_vendor_id_stays_internal_to_resume() {
        let harness = Harness::new(super::super::ConfigSource::default(), DEFAULT_MAX_SESSIONS);
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("emit.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\nprintf '{\"type\":\"system\",\"session_id\":\"VENDOR-ID-9\"}\\n'\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
        }

        let command = crate::spawn::providers::SpawnCommand {
            program: script.display().to_string(),
            args: Vec::new(),
            env: Default::default(),
            cwd: None,
            stdin_prompt: None,
        };
        let handle = harness
            .sessions
            .spawn(
                command,
                "p".into(),
                crate::config::AgentProvider::ClaudeCode,
                super::super::sessions::Provenance {
                    program: "emit".into(),
                    argv: Vec::new(),
                    env_keys: Vec::new(),
                    model: None,
                    effort: None,
                    mode: None,
                    prompt_bytes: 0,
                },
                super::super::sessions::LaunchShape::default(),
            )
            .unwrap();

        let watch = WatchOptions {
            seconds: Some(20),
            sink: None,
            cancel: tokio_util::sync::CancellationToken::new(),
        };
        harness.follow(&handle, 0, &watch).await;
        harness.harvest(&handle);

        assert_eq!(
            harness.sessions.with(&handle, |s| s.resume_token.clone()).flatten(),
            Some("VENDOR-ID-9".into()),
            "the token must still be captured — `session_send` cannot resume without it"
        );

        let (_, status) = harness.session_status(&handle).expect("status");
        let (_, list) = harness.session_list();
        let described = harness.describe(&handle, Some(true), Some(0));

        for payload in [&described, &harness.provenance(&handle), &status, &list] {
            let rendered = serde_json::to_string(payload).unwrap();
            assert!(
                !rendered.contains("vendorSessionId"),
                "the field is back on the wire: {rendered}"
            );
        }
        // Everything but `describe`, whose `text` is the vendor's own
        // event stream verbatim — the id is IN there because the vendor
        // printed it, and scrubbing a transcript would be lying about
        // what the agent emitted.
        for payload in [harness.provenance(&handle), status, list] {
            let rendered = serde_json::to_string(&payload).unwrap();
            assert!(
                !rendered.contains("VENDOR-ID-9"),
                "a vendor id reached a metadata payload: {rendered}"
            );
        }
    }

    /// The cut point is a pure function of `offset`, so a decode failure
    /// there is not transient: every later call repeats it identically.
    /// Returning "nothing new" with the cursor unmoved stalls the
    /// transcript forever and the caller sees an agent that went silent.
    #[test]
    fn read_from_advances_past_a_cap_boundary_that_splits_a_codepoint() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("turns.jsonl");
        // Lines long enough that the read cap lands mid-line, made of
        // multibyte characters so the byte cut splits a codepoint.
        let line = format!("{}\n", "é".repeat(4000));
        let body = line.repeat(20);
        assert!(body.len() > READ_CAP_BYTES, "must exceed the cap to exercise it");
        std::fs::write(&path, &body).unwrap();

        let mut offset = 0u64;
        let mut collected = String::new();
        for _ in 0..40 {
            let (chunk, _truncated, next) = read_from(&path, offset);
            if chunk.is_empty() {
                break;
            }
            assert!(next > offset, "cursor must advance or the read stalls forever");
            collected.push_str(&chunk);
            offset = next;
        }

        assert_eq!(
            collected.len(),
            body.len(),
            "every byte must eventually be read back across the cap boundary"
        );
    }

    /// A single line longer than the read cap has its newline *outside*
    /// every window, so holding for it pinned the cursor forever and the
    /// caller saw an agent that silently stopped talking. claude's
    /// `stream-json` puts one JSON object per line and a big tool result
    /// clears 60 kB routinely, so this is normal use, not a corner case.
    /// The regression this PR exists for: a transcript larger than the
    /// cap, paged by `nextOffset`, must reconstruct byte-for-byte.
    ///
    /// The old follow/spawn result kept the NEWEST `READ_CAP_BYTES`
    /// while `nextOffset` advanced past everything consumed, so the
    /// dropped prefix could never be fetched again — offsets only move
    /// forward. Paging then silently skipped the middle of a long run.
    #[test]
    fn paging_a_transcript_larger_than_the_cap_loses_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("turns.jsonl");
        // Comfortably past the cap, in realistic JSONL-ish lines.
        let body: String = (0..4000)
            .map(|i| format!("{{\"seq\":{i},\"text\":\"{}\"}}\n", "y".repeat(40)))
            .collect();
        assert!(
            body.len() > READ_CAP_BYTES * 2,
            "must exceed the cap several times over"
        );
        std::fs::write(&path, &body).unwrap();

        let mut offset = 0u64;
        let mut collected = String::new();
        for _ in 0..200 {
            let (chunk, _truncated, next) = read_from(&path, offset);
            if chunk.is_empty() {
                break;
            }
            assert!(
                chunk.ends_with('\n'),
                "every page must end on a line boundary, never mid-line"
            );
            collected.push_str(&chunk);
            offset = next;
        }

        assert_eq!(collected, body, "paging must reconstruct the transcript with no gap");
        assert_eq!(offset, body.len() as u64, "the final cursor must be EOF");
    }

    #[test]
    fn read_from_advances_through_a_line_longer_than_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("turns.jsonl");
        let huge = format!("{}\n", "x".repeat(READ_CAP_BYTES * 2 + 17));
        let body = format!("{huge}after\n");
        std::fs::write(&path, &body).unwrap();

        let mut offset = 0u64;
        let mut collected = String::new();
        for _ in 0..20 {
            let (chunk, _truncated, next) = read_from(&path, offset);
            if chunk.is_empty() {
                break;
            }
            assert!(next > offset, "cursor must advance — a stall here is unrecoverable");
            collected.push_str(&chunk);
            offset = next;
        }

        assert_eq!(collected.len(), body.len(), "the oversized line must be readable");
        assert!(
            collected.ends_with("after\n"),
            "lines after the oversized one must still arrive"
        );
    }

    /// Building the tail front-to-back and stopping at the cap meant one
    /// oversized line discarded every line after it — an empty tail for a
    /// session with plenty of readable output, and the fallback a caller
    /// reaches for when a follow returns nothing.
    #[test]
    fn tail_survives_an_oversized_line_and_keeps_the_newer_ones() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("turns.jsonl");
        let huge = "x".repeat(READ_CAP_BYTES + 500);
        std::fs::write(&path, format!("{huge}\nsecond\nthird\n")).unwrap();

        let (text, truncated) = tail_of(&path, 200);

        assert!(truncated, "dropping the oversized line must be reported");
        assert!(text.contains("second"), "newer lines must survive: {text:?}");
        assert!(text.contains("third"), "newer lines must survive: {text:?}");
        assert!(!text.contains(&huge), "the oversized line itself is dropped");
    }

    /// `with_config` folds into `ProfileConfig`, which carries flat
    /// `command` / `args` / `env` that wholesale-replace the agent's. Left
    /// open, one prompt-injected tool call is arbitrary execution on
    /// whatever host runs the sidecar.
    #[test]
    fn with_config_cannot_reach_anything_executable() {
        // The direct route.
        for key in ["command", "args", "env"] {
            assert!(
                reject_executable_overrides(&[json!({ key: "anything" })]).is_err(),
                "`{key}` must be refused — it replaces what runs"
            );
        }

        // The two that defeated an earlier deny-list of exactly those
        // three keys, and are the reason this is an allow-list now:
        //
        // `mcps` carries inline `mcp_servers` entries with their OWN
        // command/args, which the launcher writes into the vendor's MCP
        // config and the vendor then spawns. Verified reachable against a
        // running server before this was tightened.
        assert!(
            reject_executable_overrides(&[json!({
                "mcps": [{ "mcp_servers": { "x": { "command": "/bin/sh", "args": ["-c", "id"] } } }]
            })])
            .is_err(),
            "an inline MCP server is arbitrary execution by another name"
        );
        // A `$` directive addresses a field without ever naming it, so
        // key-matching alone never sees `args`.
        assert!(
            reject_executable_overrides(&[json!({ "$deleteFromPrimitiveList/args": ["--sandbox"] })]).is_err(),
            "a directive must not strip a profile's safety flags"
        );

        // Neither may `$patch`, nor a key buried in a later overlay.
        assert!(reject_executable_overrides(&[json!({ "$patch": "replace", "command": "/bin/sh" })]).is_err());
        assert!(reject_executable_overrides(&[json!({ "model": "x" }), json!({ "args": ["-c"] })]).is_err());
        // `system_prompt` reads a file from disk into the child's context.
        assert!(reject_executable_overrides(&[json!({ "system_prompt": [{ "file": "~/.ssh/id_ed25519" }] })]).is_err());

        // The one-off swap the parameter exists for stays open.
        assert!(reject_executable_overrides(&[json!({ "model": "m", "effort": "high", "mode": "plan" })]).is_ok());
        assert!(reject_executable_overrides(&[]).is_ok());
    }

    #[test]
    fn vendor_session_id_reads_all_three_vendor_keys() {
        // Each key was observed on the real CLI; they are genuinely
        // different spellings, so a single-key parser would silently
        // break resume for two of the three vendors.
        let claude = r#"{"type":"system","session_id":"40f8cb08-9426"}"#;
        let codex = r#"{"type":"thread.started","thread_id":"019fb847-a1a5"}"#;
        let opencode = r#"{"type":"error","sessionID":"ses_047b818ed"}"#;

        assert_eq!(vendor_session_id(claude).as_deref(), Some("40f8cb08-9426"));
        assert_eq!(vendor_session_id(codex).as_deref(), Some("019fb847-a1a5"));
        assert_eq!(vendor_session_id(opencode).as_deref(), Some("ses_047b818ed"));
    }

    #[test]
    fn vendor_session_id_scans_past_leading_noise() {
        let body = "not json\n{\"type\":\"warmup\"}\n{\"thread_id\":\"t-1\"}\n";
        assert_eq!(vendor_session_id(body).as_deref(), Some("t-1"));
    }

    #[test]
    fn vendor_session_id_absent_is_none_not_empty_string() {
        assert_eq!(vendor_session_id("{\"type\":\"x\"}\n"), None);
        assert_eq!(vendor_session_id("{\"session_id\":\"\"}\n"), None);
    }

    #[test]
    fn tail_returns_whole_lines_and_flags_truncation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("turns.jsonl");
        std::fs::write(&path, "a\nb\nc\nd\n").unwrap();

        let (text, truncated) = tail_of(&path, 2);
        assert_eq!(text, "c\nd\n");
        assert!(truncated, "dropping earlier lines must be reported");

        let (text, truncated) = tail_of(&path, 10);
        assert_eq!(text, "a\nb\nc\nd\n");
        assert!(!truncated);
    }

    #[test]
    fn read_from_holds_back_partial_trailing_lines() {
        // A session still being written to will have an incomplete final
        // line; handing back half a JSON object would break the caller's
        // parse, so it waits for the newline.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("turns.jsonl");
        std::fs::write(&path, "{\"a\":1}\n{\"b\":2").unwrap();

        let (text, _truncated, next) = read_from(&path, 0);
        assert_eq!(text, "{\"a\":1}\n");
        assert_eq!(next, 8, "offset resumes at the start of the partial line");

        let (text, _, _) = read_from(&path, next);
        assert_eq!(text, "", "still no complete line available");
    }

    #[test]
    fn profiles_table_marks_default_and_errors() {
        let profiles = vec![
            crate::resolve::ProfileSummary {
                id: "engineer".into(),
                provider: "claude-code",
                model: Some("opus".into()),
                is_default: true,
                ..Default::default()
            },
            crate::resolve::ProfileSummary {
                id: "broken".into(),
                provider: "codex",
                error: Some("bad patch".into()),
                ..Default::default()
            },
        ];
        let out = profiles_table(&profiles);

        assert!(out.contains("* engineer"), "{out}");
        assert!(out.contains("! broken"), "{out}");
        assert!(out.contains("bad patch"), "{out}");
        assert!(out.contains("<vendor default>"), "a model-less profile still renders");
    }

    #[test]
    fn empty_profile_list_says_so_rather_than_rendering_an_empty_table() {
        assert!(profiles_table(&[]).contains("opt-in per profile"));
    }
}

// ── SEP-2663 tasks ────────────────────────────────────────────────────

/// Separator between the session handle and the turn index in a task id.
///
/// Safe because a handle is a bare v4 UUID — hex and `-` only — so
/// `rsplit_once` is unambiguous.
const TASK_ID_SEP: char = ':';

/// Suggested poll interval handed to the client, in ms. A turn runs for
/// minutes; polling faster than this buys nothing but load.
const TASK_POLL_INTERVAL_MS: u64 = 2_000;

/// A task id names ONE TURN of one session, not the session.
///
/// The session handle alone cannot be the task id. SEP-2663 makes
/// `completed` / `failed` / `cancelled` terminal — "once reached, the
/// task's state does not change" — while a session handle is reused
/// across turns and cycles `exited → running → exited`. Keyed by handle
/// alone, turn 2 starting would mutate turn 1's finished task.
pub(crate) fn task_id(handle: &str, turn: u32) -> String {
    format!("{handle}{TASK_ID_SEP}{turn}")
}

pub(crate) fn parse_task_id(raw: &str) -> Option<(&str, u32)> {
    let (handle, turn) = raw.rsplit_once(TASK_ID_SEP)?;
    if handle.is_empty() {
        return None;
    }
    Some((handle, turn.parse().ok()?))
}

impl Harness {
    /// Current state of one task, or an error naming why it is gone.
    ///
    /// Deliberately does NOT fabricate a terminal state for a session
    /// that was evicted or reaped. A caller polling a task it can no
    /// longer resolve needs to know the record is gone, not be handed a
    /// `failed` we cannot substantiate.
    pub(crate) fn task_view(&self, raw: &str) -> Result<rmcp::model::DetailedTask, String> {
        use rmcp::model::{DetailedTask, Task, TaskPayload, TaskStatus};

        let (handle, turn) = parse_task_id(raw).ok_or_else(|| {
            format!("`{raw}` is not a task id. Task ids come from a `spawn` or `session_send` result.")
        })?;

        // Lazily, for the same reason `session_send` does it: a detached
        // turn never runs the waiting path, so nothing else would fold
        // the vendor's own id into the session.
        self.harvest(handle);

        let record = self
            .sessions
            .with(handle, |session| session.turn_record(turn))
            .ok_or_else(|| {
                format!(
                    "session `{handle}` is gone — evicted, reaped, or never existed. \
                     Its task records went with it."
                )
            })?;
        let record = record.ok_or_else(|| format!("session `{handle}` has no turn {turn}."))?;

        let payload = match record.outcome {
            TurnOutcome::Running => TaskPayload::Working,
            TurnOutcome::Killed => TaskPayload::Cancelled,
            // EVERY exit is `Completed`, including a non-zero one.
            //
            // SEP-2663 reserves `failed` for "a JSON-RPC error occurred
            // during execution" — the request itself could not be
            // fulfilled. A vendor CLI exiting 1 is not that: the harness
            // ran the agent successfully and the agent reported a normal
            // failure, which is why the non-task path already returns
            // `is_error: false` for exactly this case. Mapping it to
            // `failed` would mean inventing a JSON-RPC error, discarding
            // the transcript that explains what went wrong, and handing a
            // task-mode caller something different from what every other
            // caller gets. The exit code and the transcript are both in
            // the result; the caller can tell.
            TurnOutcome::Exited(_) => {
                // The result the same call would have returned
                // synchronously — the whole `CallToolResult`, content
                // block included, so a task-mode caller and a normal one
                // read the same bytes. A structured-only payload renders
                // as "Unknown" in opencode.
                let payload = self.describe(handle, Some(true), Some(record.transcript_from));
                let summary = super::harness_server::launch_summary(&payload);
                let mut result = rmcp::model::CallToolResult::structured(payload);
                result.content = vec![rmcp::model::ContentBlock::text(summary)];
                let serialized = serde_json::to_value(&result)
                    .ok()
                    .and_then(|v| v.as_object().cloned())
                    .unwrap_or_default();
                TaskPayload::Completed { result: serialized }
            }
        };

        let updated = record.finished_at.clone().unwrap_or_else(|| record.started_at.clone());
        let mut task = Task::new(raw.to_string(), TaskStatus::Working, record.started_at.clone(), updated);
        // Unlimited would be a lie in the other direction, but so would a
        // number: retention is bounded by `--max-sessions` eviction, by
        // `session_kill` reaping a finished session, and by the sidecar's
        // own lifetime. None of those is a duration. `None` at least does
        // not promise a window we cannot honour.
        task.ttl_ms = None;
        task.poll_interval_ms = Some(TASK_POLL_INTERVAL_MS);

        Ok(DetailedTask::new(task, payload))
    }

    /// Which turn is currently in flight for a session, if it exists.
    pub(crate) fn current_turn(&self, handle: &str) -> Option<u32> {
        self.sessions.with(handle, |session| session.turn)
    }

    /// Cancel ONE turn, addressed by task id.
    ///
    /// Deliberately NOT `session_kill`. That tool is state-aware and
    /// reaps an already-finished session — which for a protocol cancel is
    /// data loss: `tasks/cancel` on a *completed* task would delete the
    /// whole conversation, its transcript, and every other turn's record,
    /// and the completed task would stop reporting `completed`. A
    /// spec-legal call must never destroy a conversation.
    ///
    /// Cancelling a task that already reached a terminal state is a
    /// no-op, per SEP-2663: terminal states do not change. So only the
    /// turn actually in flight can be cancelled, and addressing an older
    /// turn acknowledges without touching the running one.
    pub(crate) async fn cancel_turn(&self, handle: &str, turn: u32) -> Result<(), String> {
        let current = self
            .sessions
            .with(handle, |session| session.turn)
            .ok_or_else(|| format!("session `{handle}` is gone — evicted, reaped, or never existed."))?;
        if current != turn {
            return Ok(());
        }
        // `kill` alone: terminate the process group, keep the transcript.
        self.sessions.kill(handle).await;
        Ok(())
    }

    /// Record that this turn handed out a task handle.
    ///
    /// The exit hook gates the completion push on this rather than on the
    /// peer's capabilities — see `TurnRecord::task_minted`. One recorded
    /// fact answers both questions the push has to ask: did the caller
    /// opt in, and is there a task to report on.
    fn mark_task_minted(&self, handle: &str, turn: u32) {
        self.sessions.with_mut(handle, |session| {
            if let Some(record) = session.turns.iter_mut().find(|r| r.turn == turn) {
                record.task_minted = true;
            }
        });
    }

    /// Whether this turn handed out a task handle.
    pub(crate) fn turn_minted_task(&self, handle: &str, turn: u32) -> bool {
        self.sessions
            .with(handle, |session| {
                session.turns.iter().any(|r| r.turn == turn && r.task_minted)
            })
            .unwrap_or(false)
    }

    /// Seed state for a task the caller just created.
    pub(crate) fn new_task(&self, handle: &str, turn: u32) -> rmcp::model::CreateTaskResult {
        use rmcp::model::{CreateTaskResult, MetaObject, Task, TaskStatus};

        // Record it here, at the one moment a task handle actually
        // reaches a caller. The exit hook has no other way to know.
        self.mark_task_minted(handle, turn);

        let id = task_id(handle, turn);
        let now = rmcp::task_manager::current_timestamp();
        let mut task = Task::new(id, TaskStatus::Working, now.clone(), now);
        task.ttl_ms = None;
        task.poll_interval_ms = Some(TASK_POLL_INTERVAL_MS);

        let mut result = CreateTaskResult::new(task);
        // The handle rides `_meta` so a task-mode caller never has to
        // PARSE the task id to get it. Every other tool here takes the
        // handle, and an id a caller must take apart is not opaque.
        let mut meta = serde_json::Map::new();
        meta.insert("io.hyprpilot/session".into(), json!(handle));
        result.meta = Some(MetaObject(meta));
        result
    }
}

#[cfg(test)]
mod task_tests {
    use super::*;

    /// A task names a TURN, not a session — the whole reason the id is
    /// not just the handle.
    #[test]
    fn task_ids_round_trip_and_reject_bare_handles() {
        let handle = "3b5ce010-1f61-4a8a-8fa7-086d4b5d43c0";
        let id = task_id(handle, 7);
        assert_eq!(id, format!("{handle}:7"));
        assert_eq!(parse_task_id(&id), Some((handle, 7)));

        // A bare handle is not a task id. It must not silently resolve to
        // turn 1, or a caller passing the wrong identifier gets a
        // plausible answer about the wrong thing.
        assert_eq!(parse_task_id(handle), None);
        assert_eq!(parse_task_id(""), None);
        assert_eq!(parse_task_id(":3"), None, "an empty handle is not addressable");
        assert_eq!(parse_task_id(&format!("{handle}:x")), None);
    }

    /// The separator has to survive the handle's own alphabet. A v4 UUID
    /// is hex and `-` only, so `rsplit_once(':')` cannot be fooled by the
    /// handle — this pins that assumption rather than trusting it.
    #[test]
    fn the_separator_cannot_collide_with_a_handle() {
        let handle = uuid::Uuid::new_v4().to_string();
        assert!(
            !handle.contains(':'),
            "handles must not contain the task-id separator: {handle}"
        );
        let id = task_id(&handle, 12);
        let (parsed, turn) = parse_task_id(&id).expect("round trip");
        assert_eq!(parsed, handle);
        assert_eq!(turn, 12);
    }

    /// Cancelling a TERMINAL task must not touch the conversation.
    ///
    /// The bug this pins was live: `tasks/cancel` routed through
    /// `session_kill`, which reaps an already-finished session — so a
    /// spec-legal cancel of a completed turn-1 task killed turn 2 and
    /// deleted the transcript, and the completed task stopped reporting
    /// `completed`. Terminal states are immutable; cancelling one is a
    /// no-op, not a licence to destroy data.
    #[tokio::test]
    async fn cancelling_a_finished_turn_leaves_the_session_intact() {
        let harness = Harness::new(super::super::ConfigSource::default(), DEFAULT_MAX_SESSIONS);
        let handle = harness
            .sessions
            .spawn(
                crate::spawn::providers::SpawnCommand {
                    program: "/bin/sh".into(),
                    args: vec!["-c".into(), "exit 0".into()],
                    env: Default::default(),
                    cwd: None,
                    stdin_prompt: None,
                },
                "p".into(),
                crate::config::AgentProvider::ClaudeCode,
                super::super::sessions::Provenance {
                    program: "sh".into(),
                    argv: Vec::new(),
                    env_keys: Vec::new(),
                    model: None,
                    effort: None,
                    mode: None,
                    prompt_bytes: 0,
                },
                super::super::sessions::LaunchShape::default(),
            )
            .unwrap();
        let mut done = harness.sessions.with(&handle, |s| s.completion()).unwrap();
        while done.borrow().is_none() {
            done.changed().await.unwrap();
        }

        // Turn 1 is finished. Cancelling it addresses a terminal task.
        harness
            .cancel_turn(&handle, 1)
            .await
            .expect("a terminal cancel is a no-op");
        assert!(
            harness.sessions.with(&handle, |s| s.turn).is_some(),
            "the session must survive — reaping it here destroyed the whole conversation"
        );
        assert!(
            harness.task_view(&task_id(&handle, 1)).is_ok(),
            "a completed task must keep reporting after a cancel it should have ignored"
        );

        // An older turn's id must never reach a later turn.
        harness
            .cancel_turn(&handle, 99)
            .await
            .expect("an unknown turn is a no-op");
        assert!(harness.sessions.with(&handle, |s| s.turn).is_some());

        // An unresolvable handle is an error, matching `tasks/get`.
        assert!(harness.cancel_turn("nope", 1).await.is_err());
    }

    /// The completion push is gated on this flag, so a mint that fails
    /// to record it means a task-mode caller polls forever and is never
    /// told its turn ended. Nothing else in the suite covers it — the
    /// gap was caught by clippy noticing the setter had no caller, which
    /// is not a guarantee that survives a refactor.
    #[tokio::test]
    async fn minting_a_task_records_it_on_the_turn() {
        let harness = Harness::new(super::super::ConfigSource::default(), DEFAULT_MAX_SESSIONS);
        let command = crate::spawn::providers::SpawnCommand {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), "exit 0".into()],
            env: Default::default(),
            cwd: None,
            stdin_prompt: None,
        };
        let handle = harness
            .sessions
            .spawn(
                command,
                "p".into(),
                crate::config::AgentProvider::ClaudeCode,
                super::super::sessions::Provenance {
                    program: "sh".into(),
                    argv: Vec::new(),
                    env_keys: Vec::new(),
                    model: None,
                    effort: None,
                    mode: None,
                    prompt_bytes: 0,
                },
                super::super::sessions::LaunchShape::default(),
            )
            .unwrap();

        assert!(
            !harness.turn_minted_task(&handle, 1),
            "a launch that never handed out a task must not look like one that did"
        );
        let created = harness.new_task(&handle, 1);
        assert_eq!(created.task.task_id, task_id(&handle, 1));
        assert!(
            harness.turn_minted_task(&handle, 1),
            "minting must record on the turn, or the completion push never fires"
        );
        // The handle must be reachable WITHOUT parsing the task id.
        let meta = created.meta.expect("_meta carries the session handle");
        assert_eq!(
            meta.0.get("io.hyprpilot/session").and_then(|v| v.as_str()),
            Some(handle.as_str())
        );
    }

    /// An unknown or evicted session must produce an ERROR, never a
    /// fabricated terminal state. A caller polling a task it can no
    /// longer resolve needs to know the record is gone — reporting
    /// `failed` would be a status we cannot substantiate.
    #[test]
    fn an_unresolvable_task_errors_rather_than_inventing_a_status() {
        let harness = Harness::new(super::super::ConfigSource::default(), DEFAULT_MAX_SESSIONS);
        let err = harness
            .task_view("3b5ce010-1f61-4a8a-8fa7-086d4b5d43c0:1")
            .expect_err("an unknown session cannot resolve");
        assert!(err.contains("gone"), "the error must say the record is gone: {err}");

        let err = harness.task_view("not-a-task-id").expect_err("garbage cannot resolve");
        assert!(err.contains("not a task id"), "got: {err}");
    }
}
