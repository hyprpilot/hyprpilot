//! The agent harness — `list_profiles` / `spawn` / `session_send` /
//! `session_list` / `session_read` / `session_kill`.
//!
//! Turns the captain's profile registry into something a connected
//! agent can drive: pick a profile, run it, talk to it across turns,
//! read its transcript, kill it. Gated behind `--with-harness` because
//! `auto_inject` puts this sidecar inside *every* launch, and an
//! ungated spawn surface would let a claude session spawn nested claude
//! sessions without bound.
//!
//! Every launch flows through `spawn::prepare`, the same path
//! `hyprpilot <profile>` uses, so prompt-source priority, the
//! `-- <args>` escape hatch, and cwd precedence cannot drift between
//! the CLI and this surface.

use std::sync::Arc;

use serde_json::{json, Value};

use super::sessions::{SessionStatus, SessionTable};
use super::ConfigSource;
use crate::spawn::providers::HarnessProjection;
use crate::spawn::{LaunchOrigin, SpawnRequest};

/// Env stamp bounding recursive spawning. A spawned agent gets
/// `depth + 1`; past [`MAX_SPAWN_DEPTH`] a spawn is refused.
pub(crate) const DEPTH_ENV: &str = "HYPRPILOT_SPAWN_DEPTH";
const MAX_SPAWN_DEPTH: usize = 2;

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
const DEFAULT_TAIL_LINES: usize = 200;
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
/// `dropped_earlier` matters: the accumulator is bounded to
/// [`READ_CAP_BYTES`], so a long run's early output is discarded as it
/// goes. Saying so beats returning a silently-partial transcript.
pub(crate) struct FollowResult {
    pub text: String,
    /// Byte offset to resume from — handed back as `nextOffset` so the
    /// documented poll-after-timeout workflow actually has one.
    pub cursor: u64,
    pub finished: bool,
    pub dropped_earlier: bool,
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
                "spawn refused: nesting depth {} has reached the limit of {MAX_SPAWN_DEPTH}. \
                 This session was itself spawned by a hyprpilot harness.",
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

        let projection = HarnessProjection {
            structured_output: true,
            resume: resume.as_ref().map(|target| target.vendor_session_id.clone()),
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
            return Ok(self.describe(&handle, None, None, Some(resume_from)));
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

        Ok(self.describe(&handle, Some(finished), Some(followed.text), Some(followed.cursor)))
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
            return FollowResult {
                text: String::new(),
                cursor: from,
                finished: true,
                dropped_earlier: false,
            };
        };

        let deadline = watch
            .seconds
            .map(|secs| tokio::time::Instant::now() + std::time::Duration::from_secs(secs));
        let mut streamed = String::new();
        let mut cursor = from;
        let mut dropped_earlier = false;

        loop {
            let (chunk, _truncated, next) = read_from(&turns, cursor);
            if !chunk.is_empty() {
                if let Some(sink) = watch.sink.as_ref() {
                    sink.push(&chunk, next).await;
                }
                streamed.push_str(&chunk);
                // Keep only what we can actually return. A long run emits
                // hundreds of MB of stream-json and every byte past the
                // cap is discarded at the end anyway — accumulating it
                // all drove the sidecar's RSS to match the transcript,
                // and an OOM-kill takes every live session with it.
                if streamed.len() > READ_CAP_BYTES {
                    streamed = tail_bytes(&streamed, READ_CAP_BYTES).to_string();
                    dropped_earlier = true;
                }
                cursor = next;
                // Keep draining while output flows; only park once the
                // file has nothing new.
                continue;
            }
            if completion.borrow().is_some() {
                return FollowResult {
                    text: streamed,
                    cursor,
                    finished: true,
                    dropped_earlier,
                };
            }
            if watch.cancel.is_cancelled() {
                tracing::debug!(%handle, "mcp harness: follow ended by client cancellation");
                return FollowResult {
                    text: streamed,
                    cursor,
                    finished: false,
                    dropped_earlier,
                };
            }
            if deadline.is_some_and(|deadline| tokio::time::Instant::now() >= deadline) {
                return FollowResult {
                    text: streamed,
                    cursor,
                    finished: false,
                    dropped_earlier,
                };
            }

            tokio::select! {
                _ = watch.cancel.cancelled() => {
                    tracing::debug!(%handle, "mcp harness: follow ended by client cancellation");
                    return FollowResult { text: streamed, cursor, finished: false, dropped_earlier };
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
            if session.vendor_session_id.is_none() {
                session.vendor_session_id = Some(id.clone());
            }
            session.last_turn_at = std::time::SystemTime::now();
        });
    }

    /// The result shape shared by `spawn` and `session_send`. `streamed` is
    /// what the follow already read, reused so the result and the
    /// progress notifications tell the same story.
    fn describe(
        &self,
        handle: &str,
        finished: Option<bool>,
        streamed: Option<String>,
        next_offset: Option<u64>,
    ) -> Value {
        let Some(snapshot) = self.sessions.with(handle, |session| {
            (
                session.profile_id.clone(),
                session.provider.wire_id(),
                session.status(),
                session.exit_code(),
                session.vendor_session_id.clone(),
                session.turns_path(),
            )
        }) else {
            return json!({ "session": handle, "status": "exited" });
        };
        let (profile, provider, status, exit_code, vendor, turns) = snapshot;

        let mut out = json!({
            "session": handle,
            "profile": profile,
            "provider": provider,
            "status": status.as_str(),
        });
        if let Some(code) = exit_code {
            out["exitCode"] = json!(code);
        }
        if let Some(vendor) = vendor {
            out["vendorSessionId"] = json!(vendor);
        }
        if finished == Some(false) {
            out["timedOut"] = json!(true);
        }
        out["sessionInfo"] = self.provenance(handle);
        // The offset a caller needs to poll from after a timed-out spawn.
        // Its absence made the documented "poll `session_read { session,
        // offset }`" workflow impossible to follow — there was no offset
        // to pass, so a caller had to fall back to `tail` and re-read
        // what it had already seen.
        if let Some(next) = next_offset {
            out["nextOffset"] = json!(next);
        }
        match streamed {
            Some(streamed) => {
                let truncated = streamed.len() > READ_CAP_BYTES;
                let body = tail_bytes(&streamed, READ_CAP_BYTES).to_string();
                out["text"] = json!(body);
                out["truncated"] = json!(truncated);
            }
            None if status == SessionStatus::Exited => {
                let (text, truncated) = tail_of(&turns, DEFAULT_TAIL_LINES);
                out["text"] = json!(text);
                out["truncated"] = json!(truncated);
            }
            None => {}
        }

        out
    }

    pub(crate) fn list_profiles(&self) -> Result<(String, Value), String> {
        let cfg = self
            .config
            .load()
            .map_err(|err| format!("could not load hyprpilot config: {err:#}"))?;
        let profiles = crate::spawn::list_profiles(&cfg, None, &[]);
        let summary = profiles_table(&profiles);

        Ok((summary, json!({ "profiles": profiles })))
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
        // Harvest lazily. A `spawn { wait: false }` returns before the
        // waiting path ever runs, so its session has no vendor id yet —
        // without this it could never be resumed, and the "never
        // reported a vendor session id" branch below would blame a
        // failed first turn for a session that is perfectly healthy.
        self.harvest(handle);

        let target = self
            .sessions
            .with(handle, |session| ResumeTarget {
                handle: session.handle.clone(),
                profile_id: session.profile_id.clone(),
                vendor_session_id: session.vendor_session_id.clone().unwrap_or_default(),
                has_vendor_id: session.vendor_session_id.is_some(),
            })
            .ok_or_else(|| format!("unknown session `{handle}`. Call `session_list` for live handles."))?;

        if !target.has_vendor_id {
            return Err(format!(
                "session `{handle}` never reported a vendor session id, so it cannot be resumed. \
                 Its first turn probably failed before the agent started — check `session_read`."
            ));
        }

        // The "still running" refusal lives in `SessionTable::respawn`,
        // under the table lock. Checking it here as well would only be a
        // second chance to lose the race — the lock is what actually
        // guarantees one turn at a time.
        let args = LaunchToolArgs {
            profile: target.profile_id.clone(),
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
                    "cwd": session.cwd.as_ref().map(|c| c.display().to_string()),
                    "startedAt": unix_secs(session.created_at),
                    "lastTurnAt": unix_secs(session.last_turn_at),
                    "vendorSessionId": session.vendor_session_id,
                    "pid": session.pid(),
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
            if let Some(cwd) = session.cwd.as_ref() {
                row["cwd"] = json!(cwd.display().to_string());
            }
            if let Some(vendor) = session.vendor_session_id.as_ref() {
                row["vendorSessionId"] = json!(vendor);
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
        offset: Option<u64>,
        watch: Option<WatchOptions>,
    ) -> Result<Value, String> {
        let paths = self
            .sessions
            .with(handle, |session| {
                (session.turns_path(), session.stderr_path(), session.status())
            })
            .ok_or_else(|| format!("unknown session `{handle}`. Call `session_list` for live handles."))?;
        let (turns, stderr_path, mut status) = paths;

        // Following streams from `offset` forward; a tail-follow would
        // have no stable resume point.
        let followed = watch.is_some();
        let mut streamed = String::new();
        let mut cursor = offset.unwrap_or(0);
        let mut dropped_earlier = false;
        if let Some(watch) = watch {
            let result = self.follow(handle, cursor, &watch).await;
            streamed = result.text;
            cursor = result.cursor;
            dropped_earlier = result.dropped_earlier;
            status = self.sessions.with(handle, |s| s.status()).unwrap_or(status);
        }

        let (text, truncated, next_offset) = match (followed, offset) {
            // A follow already consumed forward from `offset`; hand back
            // exactly what was streamed so the result and the
            // notifications agree.
            (true, _) => {
                // `dropped_earlier` is the honest half: a long follow
                // discards output as it goes to stay bounded, so
                // "truncated" is true even when what we return fits.
                let truncated = dropped_earlier || streamed.len() > READ_CAP_BYTES;
                let body = tail_bytes(&streamed, READ_CAP_BYTES).to_string();
                (body, truncated, cursor)
            }
            (false, Some(offset)) => read_from(&turns, offset),
            (false, None) => {
                let (text, truncated) = tail_of(&turns, tail);
                let end = std::fs::metadata(&turns).map(|m| m.len()).unwrap_or(0);
                (text, truncated, end)
            }
        };

        let mut out = json!({
            "session": handle,
            "status": status.as_str(),
            "lines": text,
            "nextOffset": next_offset,
            "truncated": truncated,
            "sessionInfo": self.provenance(handle),
        });
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
    vendor_session_id: String,
    has_vendor_id: bool,
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

/// Restrict a caller-supplied overlay to settings that cannot change
/// what gets executed.
///
/// The `--with-harness` gate is a grant of *the captain's profiles*, not
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

/// Last `cap` bytes of `text`, backed off to a char boundary.
///
/// **Never byte-slice a transcript.** It is model output, so a multibyte
/// codepoint straddling the cut is ordinary, and `&s[i..]` on a
/// non-boundary panics — which under the release profile's
/// `panic = "abort"` would abort the whole sidecar and take every live
/// session with it, rather than failing one tool call.
fn tail_bytes(text: &str, cap: usize) -> &str {
    if text.len() <= cap {
        return text;
    }
    let mut start = text.len() - cap;
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }

    &text[start..]
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
fn profiles_table(profiles: &[crate::resolve::ProfileSummary]) -> String {
    if profiles.is_empty() {
        return "No profiles configured. Add `[[profiles]]` entries to the hyprpilot config.".into();
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
        out.push_str(&format!("  mcps={} skills={}", profile.mcp_count, profile.skills_count));
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
            )
            .unwrap();

        let watch = WatchOptions {
            seconds: Some(20),
            sink: None,
            cancel: tokio_util::sync::CancellationToken::new(),
        };
        let result = harness.follow(&handle, 0, &watch).await;

        assert!(result.finished, "follow must report the session finished");
        assert!(result.cursor > 0, "follow must report where to resume from");
        for expected in ["one", "two", "three"] {
            assert!(
                result.text.contains(expected),
                "follow stopped early — missing {expected:?} in {:?}",
                result.text
            );
        }
    }

    /// Transcripts are model output, so a multibyte codepoint straddling
    /// the cap is ordinary. Byte-slicing there panics — and under the
    /// release profile's `panic = "abort"` that aborts the whole sidecar
    /// and kills every live session, rather than failing one tool call.
    #[test]
    fn tail_never_splits_a_codepoint() {
        // Every offset into a 3-byte-per-char string, so the cut lands
        // mid-codepoint at two of every three caps.
        let text = "日本語".repeat(50);
        for cap in 1..text.len() {
            let tail = tail_bytes(&text, cap);
            assert!(tail.len() <= cap, "tail must respect the cap");
            assert!(text.ends_with(tail), "tail must be a suffix of the input");
        }
        assert_eq!(tail_bytes("short", 100), "short", "under the cap, unchanged");
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
        assert!(profiles_table(&[]).contains("No profiles configured"));
    }
}
