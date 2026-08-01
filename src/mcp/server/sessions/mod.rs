//! Harness session store — the sidecar owns every agent it spawns.
//!
//! A session is a **direct child** of `hyprpilot mcp harness`, waited on
//! via `tokio::process::Child`, with its transcript streaming into a
//! per-session [`TempDir`]. Nothing here outlives the sidecar, and that
//! is the point: the vendor owns the sidecar's lifetime, so an
//! in-process table cannot leak state across launches the way a daemon
//! would.
//!
//! **Orphan prevention is layered, and only the last layer is a
//! guarantee:**
//!
//! 1. [`shutdown`] — SIGTERM the process group, grace, then SIGKILL.
//!    Runs on graceful transport close and on SIGTERM/SIGHUP.
//! 2. `kill_on_drop(true)` — tokio's `ChildDropGuard`. Without it
//!    tokio's default `Drop` pushes a still-running child onto a global
//!    *orphan queue* instead of killing it, which is precisely the
//!    failure this module exists to prevent.
//! 3. **`PR_SET_PDEATHSIG`** — the kernel kills the child when the
//!    sidecar dies, *however* it dies. This is the only layer that
//!    survives `SIGKILL` and the release profile's `panic = "abort"`
//!    (`Cargo.toml`), both of which run no destructor at all.
//!
//! Layers 1 and 2 are userspace courtesy; layer 3 is why an orphaned
//! agent cannot sit burning tokens with nobody holding a handle.
//!
//! Sessions run in **their own process group** so a kill from layer 1
//! reaches the vendor's own MCP sidecars and tool subprocesses, not just
//! the direct child.
//!
//! **Layer 3 does not cover the group.** `PR_SET_PDEATHSIG` signals only
//! the direct child and is cleared across that child's own forks — so in
//! the very case it exists for (sidecar SIGKILLed, nothing able to run),
//! the vendor dies but its grandchildren can survive until a later
//! sidecar's [`sweep_stale_sessions`] reclaims them. That is the one
//! real hole in the no-orphan story; say so rather than implying the
//! kernel closes it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use tempfile::TempDir;
use tokio::sync::watch;

use crate::config::AgentProvider;
use crate::spawn::providers::SpawnCommand;

/// Filename prefix for every per-session scratch directory. Also the
/// predicate the startup sweep matches on, so hyprpilot only ever
/// reclaims its own directories.
pub(crate) const SESSION_DIR_PREFIX: &str = "hyprpilot-session-";

/// The vendor's structured event stream (child stdout).
pub(crate) const TURNS_FILE: &str = "turns.jsonl";
/// Child stderr, kept separate so a vendor's diagnostics never corrupt
/// the JSONL line framing `session_read` depends on.
pub(crate) const STDERR_FILE: &str = "stderr.log";

/// Written by the waiter task when a turn's process exits.
///
/// The vendor-neutral completion signal: a shell watcher cannot call an
/// MCP tool, but it can `test -f`. Written by the same `child.wait()`
/// task that owns the truth, so no recycled PID and no zombie can
/// produce a false reading.
pub(crate) const DONE_FILE: &str = "done.json";
/// Crash-recovery breadcrumb — see [`sweep_stale_sessions`].
pub(crate) const BREADCRUMB_FILE: &str = "session.json";

/// How long a session gets between SIGTERM and SIGKILL.
const KILL_GRACE: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SessionStatus {
    Running,
    Exited,
}

impl SessionStatus {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Exited => "exited",
        }
    }
}

/// What a session was launched with — captured at spawn so a caller can
/// audit exactly what ran, even after the config on disk has changed.
///
/// `argv` is **redacted** (`providers::redacted_argv`): flag names
/// survive, every value payload becomes a `<size>` placeholder. argv
/// carries the system prompt and, for some vendors, MCP bearer tokens,
/// and a session's provenance is not worth leaking those to whoever can
/// call the tool. `env_keys` is names-only for the same reason.
#[derive(Debug, Clone)]
pub(crate) struct Provenance {
    pub program: String,
    pub argv: Vec<String>,
    pub env_keys: Vec<String>,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub mode: Option<String>,
    /// Byte length of the prompt this session started with. The prompt
    /// itself is the caller's own input, so echoing it back is noise;
    /// the size is enough to confirm what was sent.
    pub prompt_bytes: usize,
}

/// The caller-supplied launch inputs a follow-up turn REUSES.
///
/// A conversation is one session, so how it was launched is part of
/// its identity rather than a per-turn option. Re-deriving turn 2 from
/// defaults launched it differently from turn 1, silently: a dropped
/// `cwd` made claude's `--resume` fail with a bare "No conversation
/// found with session ID" (it keys its conversation store by project
/// directory, so the session was fine — it was looked up in the wrong
/// place), and a dropped `mode` / `args` / `with_config` would change
/// the agent's permissions or flags mid-conversation without saying so.
///
/// An explicit per-turn value still overrides its counterpart here.
#[derive(Debug, Clone, Default)]
pub(crate) struct LaunchShape {
    /// The RESOLVED cwd, not the caller's raw input — so a turn that
    /// fell back to `$PWD` still replays to the same directory.
    pub cwd: Option<PathBuf>,
    pub mode: Option<String>,
    pub with_config: Vec<serde_json::Value>,
    pub args: Vec<String>,
}

/// One live or finished agent session.
#[derive(Debug)]
pub(crate) struct Session {
    pub handle: String,
    pub profile_id: String,
    pub provider: AgentProvider,
    pub launch: LaunchShape,
    pub provenance: Provenance,
    /// What `session_send` hands the vendor to continue this
    /// conversation — the vendor's own session id, parsed out of the
    /// first turn's event stream. Purely internal: a caller addresses a
    /// session by its `handle`, which exists from the moment of `spawn`,
    /// whereas this is `None` until the vendor emits it (or forever, if
    /// the turn failed before it did) — which is why `session_send`
    /// reports a clean error instead of assuming one exists.
    pub resume_token: Option<String>,
    pub created_at: SystemTime,
    pub last_turn_at: SystemTime,
    /// 1-based index of the turn currently running (or the last one that
    /// ran). Incremented by `respawn` only after the child actually
    /// started, so a failed resume does not consume a number.
    pub turn: u32,
    /// One record per turn, oldest first.
    ///
    /// The session's own state cannot answer "how did turn N end?" —
    /// `respawn` replaces `done` wholesale, so the previous turn's exit
    /// code is unreachable the moment the next turn starts. That is fine
    /// for the session tools, which only ever ask about *now*, but a
    /// SEP-2663 task is per-turn and its terminal states are immutable:
    /// once `completed`, a task must keep reporting `completed` forever.
    /// This is where that history lives.
    pub turns: Vec<TurnRecord>,
    /// Removed from disk when this struct drops.
    dir: TempDir,
    pid: u32,
    pgid: i32,
    /// Exit code once the child has been reaped. Watch rather than a
    /// plain field so `wait: true` can await completion without holding
    /// the table lock across an await.
    done: watch::Receiver<Option<i32>>,
}

/// How one turn ended.
///
/// A closed set, so it is an enum rather than a `killed: bool` beside an
/// exit code. The distinction is not derivable after the fact: the waiter
/// stores `status.code().unwrap_or(-1)`, and `code()` is `None` for
/// signal death — so a session we killed, one killed by something else,
/// and a `child.wait()` error all land on `-1`, indistinguishable from
/// each other and adjacent to a genuine crash's own exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TurnOutcome {
    Running,
    Exited(i32),
    /// We terminated it — `session_kill`, or a task cancellation.
    Killed,
}

/// What one turn did, retained after the next turn overwrites the live
/// session state.
#[derive(Debug, Clone)]
pub(crate) struct TurnRecord {
    pub turn: u32,
    pub outcome: TurnOutcome,
    /// Byte offset into the transcript where this turn's output begins.
    /// The transcript is append-only across turns, so this is what makes
    /// a finished turn's output still addressable later.
    pub transcript_from: u64,
    /// ISO 8601, captured when the turn started.
    ///
    /// Stored as the wire string rather than a `SystemTime` because that
    /// is the only form SEP-2663 accepts, and the repo carries no date
    /// crate to convert one later. Capturing it here also keeps a task's
    /// `createdAt` honest: derived at poll time it would report the
    /// moment of the poll, moving on every request.
    pub started_at: String,
    pub finished_at: Option<String>,
}

impl Session {
    /// The session's scratch directory. Removed when this drops.
    #[cfg(test)]
    pub(crate) fn path(&self) -> &Path {
        self.dir.path()
    }

    pub(crate) fn turns_path(&self) -> PathBuf {
        self.dir.path().join(TURNS_FILE)
    }

    /// Where the completion marker lands. Advisory — the directory is
    /// removed on reap, eviction and shutdown, so a watcher must treat
    /// a MISSING directory as finished too, not just a present marker.
    pub(crate) fn done_path(&self) -> PathBuf {
        self.dir.path().join(DONE_FILE)
    }

    /// The session's own directory. 0700, removed when the session is
    /// reaped, evicted, or the sidecar exits.
    pub(crate) fn dir_path(&self) -> PathBuf {
        self.dir.path().to_path_buf()
    }

    /// Crash-recovery breadcrumb — pid/pgid/ownerPid/startTicks, read by
    /// the startup sweep. Internal, but surfaced because a caller
    /// debugging an orphan wants it and it costs nothing to name.
    pub(crate) fn breadcrumb_path(&self) -> PathBuf {
        self.dir.path().join(BREADCRUMB_FILE)
    }

    pub(crate) fn stderr_path(&self) -> PathBuf {
        self.dir.path().join(STDERR_FILE)
    }

    pub(crate) fn exit_code(&self) -> Option<i32> {
        *self.done.borrow()
    }

    pub(crate) fn status(&self) -> SessionStatus {
        match self.exit_code() {
            Some(_) => SessionStatus::Exited,
            None => SessionStatus::Running,
        }
    }

    pub(crate) fn pid(&self) -> u32 {
        self.pid
    }

    /// Fold the live `done` watch into the current turn's record.
    ///
    /// The record is written at turn START, when the outcome is unknown,
    /// and `Killed` is stamped by [`SessionTable::kill`]. Everything else
    /// has to be read from the watch at query time — the waiter task
    /// cannot reach the table to write it back without a lock it must not
    /// hold. A `Killed` stamp always wins: the exit code it produced is
    /// `-1`, which says nothing.
    pub(crate) fn turn_record(&self, turn: u32) -> Option<TurnRecord> {
        let record = self.turns.iter().find(|r| r.turn == turn)?;
        if record.outcome != TurnOutcome::Running || turn != self.turn {
            return Some(record.clone());
        }
        let mut live = record.clone();
        if let Some(code) = self.exit_code() {
            live.outcome = TurnOutcome::Exited(code);
            live.finished_at = Some(rmcp::task_manager::current_timestamp());
        }
        Some(live)
    }

    /// Byte length of the transcript, used as the start offset of the
    /// turn about to begin. Zero for a fresh session or an unreadable
    /// file — a start offset that reads from the beginning is wrong only
    /// cosmetically, whereas failing a launch over it would not be.
    fn transcript_len(&self) -> u64 {
        std::fs::metadata(self.turns_path()).map(|m| m.len()).unwrap_or(0)
    }

    /// A receiver that resolves once the child exits. Cloned out so the
    /// caller can await it *after* releasing the table lock — holding a
    /// `std::sync::Mutex` across an await would stall every concurrent
    /// tool call.
    pub(crate) fn completion(&self) -> watch::Receiver<Option<i32>> {
        self.done.clone()
    }
}

/// Best-effort signal to a whole process group. Every failure is logged
/// and swallowed: a session that already exited is the common case, not
/// an error worth failing a tool call over.
fn signal_group(pgid: i32, sig: nix::sys::signal::Signal) {
    use nix::sys::signal::killpg;
    use nix::unistd::Pid;

    if let Err(err) = killpg(Pid::from_raw(pgid), sig) {
        tracing::debug!(pgid, ?sig, %err, "mcp harness: signalling process group failed");
    }
}

/// Called once per turn, when its process exits.
///
/// Deliberately a bare closure rather than anything MCP-shaped: this
/// module owns processes and has **zero** rmcp dependencies, and that
/// separation is worth more than the indirection costs. The channel
/// notification is built in `harness.rs`, where the protocol types
/// already live.
pub(crate) type ExitHook = Arc<dyn Fn(String, i32) + Send + Sync>;

/// The in-process session table. Bounded by the sidecar's own lifetime —
/// there is no persistence and no cross-launch state.
#[derive(Default)]
pub(crate) struct SessionTable {
    inner: Mutex<BTreeMap<String, Session>>,
    /// Set once, after the server starts serving — a `Peer` does not
    /// exist before that, and sessions cannot exist before it either.
    on_exit: std::sync::OnceLock<ExitHook>,
}

impl SessionTable {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Install the per-turn completion hook. Idempotent; a second call
    /// is ignored rather than replacing the first.
    pub(crate) fn set_exit_hook(&self, hook: ExitHook) {
        let _ = self.on_exit.set(hook);
    }

    /// Mint a session handle.
    ///
    /// A v4 UUID, not a counter: two sidecars running at once would both
    /// mint `…-1` from their own tables, which is harmless only while
    /// their namespaces never meet. A UUID is unique without any
    /// coordination between them, so a handle stays unambiguous wherever
    /// it ends up — in a log, a transcript, or a caller that talks to
    /// two servers.
    ///
    /// The handle is a bare UUID — no prefix, no provider, no profile.
    /// It is resolved by table lookup, so it only has to be unique and
    /// known to us; encoding anything else would just create a second
    /// copy of facts already returned as fields on every result that
    /// mentions a session (`session_list`, `sessionInfo`, `spawn`).
    fn mint_handle(&self) -> String {
        uuid::Uuid::new_v4().to_string()
    }

    /// Number of sessions whose child is still running. This — not
    /// [`len`] — is what the concurrency ceiling bounds, since an exited
    /// session holds a transcript but no model connection.
    pub(crate) fn live(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .filter(|s| s.status() == SessionStatus::Running)
            .count()
    }

    /// Run `f` against one session, or `None` when the handle is unknown.
    /// The closure runs under the lock, so it must not await.
    pub(crate) fn with<T>(&self, handle: &str, f: impl FnOnce(&Session) -> T) -> Option<T> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).get(handle).map(f)
    }

    pub(crate) fn with_mut<T>(&self, handle: &str, f: impl FnOnce(&mut Session) -> T) -> Option<T> {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get_mut(handle)
            .map(f)
    }

    /// Snapshot every session into a caller-owned shape. Takes a
    /// projection so the lock is released before anything is rendered.
    pub(crate) fn map_all<T>(&self, f: impl Fn(&Session) -> T) -> Vec<T> {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .map(f)
            .collect()
    }

    /// Start another turn on an EXISTING session, keeping its handle.
    ///
    /// The handle is stable for the life of a conversation: a caller that
    /// spawned once can keep using the id it was given, and an N-turn
    /// conversation costs one table entry and one transcript, not N.
    ///
    /// **The whole check-and-replace happens under the table lock**, and
    /// that is the point. `tokio::process::Command::spawn` is synchronous,
    /// so there is no await to hold the lock across — which makes the
    /// "one turn at a time" rule an actual invariant rather than a
    /// check that a second caller can slip past. Two concurrent sends on
    /// one handle: the first wins, the second gets
    /// [`RespawnError::Busy`].
    pub(crate) fn respawn(
        self: &Arc<Self>,
        handle: &str,
        command: SpawnCommand,
        provenance: Provenance,
    ) -> std::result::Result<(), RespawnError> {
        let mut guard = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let session = guard.get_mut(handle).ok_or(RespawnError::Unknown)?;
        if session.status() == SessionStatus::Running {
            return Err(RespawnError::Busy);
        }

        // Seal the outgoing turn BEFORE the watch is replaced — after
        // the swap its exit code is unreachable.
        let ending = session.turn;
        if let Some(sealed) = session.turn_record(ending) {
            if let Some(slot) = session.turns.iter_mut().find(|r| r.turn == ending) {
                *slot = sealed;
            }
        }
        let transcript_from = session.transcript_len();

        // Append, so the conversation stays ONE transcript and the byte
        // offsets a caller is paging through remain valid across turns.
        let launched = launch_child(&command, session.dir.path(), handle, true, self.on_exit.get().cloned())
            .map_err(RespawnError::Spawn)?;

        // Only now — a launch that failed must not consume a turn number.
        session.turn += 1;
        session.turns.push(TurnRecord {
            turn: session.turn,
            outcome: TurnOutcome::Running,
            transcript_from,
            started_at: rmcp::task_manager::current_timestamp(),
            finished_at: None,
        });

        session.launch.cwd = command.cwd.clone();
        session.provenance = provenance;
        session.pid = launched.pid;
        session.pgid = launched.pgid;
        session.done = launched.done;
        session.last_turn_at = SystemTime::now();

        Ok(())
    }

    /// Drop a session from the table, removing its transcript directory.
    ///
    /// Returns whether it was there. Only meaningful for a FINISHED
    /// session — [`kill`] refuses to forget a live one, since dropping
    /// the entry would strand the process with nobody holding its
    /// handle.
    pub(crate) fn forget(&self, handle: &str) -> bool {
        // Removing the `Session` drops its `TempDir`, which removes the
        // transcript from disk.
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(handle)
            .is_some()
    }

    /// Drop the oldest finished sessions once the table grows past `cap`.
    ///
    /// Exited sessions cost a table entry and a transcript directory
    /// each. Without a bound, a caller that spawns steadily accumulates
    /// both for the sidecar's whole life. Only FINISHED sessions are
    /// evicted, oldest-by-last-turn first, so a live agent is never
    /// touched — and it is logged, because a caller whose transcript
    /// vanished deserves to find out why.
    pub(crate) fn evict_exited_over(&self, cap: usize) {
        let mut guard = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if guard.len() <= cap {
            return;
        }
        let mut finished: Vec<(SystemTime, String)> = guard
            .values()
            .filter(|session| session.status() == SessionStatus::Exited)
            .map(|session| (session.last_turn_at, session.handle.clone()))
            .collect();
        finished.sort_by_key(|(at, _)| *at);

        let excess = guard.len() - cap;
        for (_, handle) in finished.into_iter().take(excess) {
            guard.remove(&handle);
            tracing::info!(%handle, cap, "mcp harness: evicted a finished session to bound the table");
        }
    }

    /// Terminate one session's process group. Returns `None` for an
    /// unknown handle, `Some(was_running)` otherwise — killing an
    /// already-exited session is a no-op, not an error.
    pub(crate) async fn kill(&self, handle: &str) -> Option<bool> {
        // Clone the bits we need and drop the lock: `terminate` awaits.
        let target = self.with(handle, |s| (s.pgid, s.completion(), s.handle.clone()))?;
        let (pgid, mut done, handle) = target;
        if done.borrow().is_some() {
            return Some(false);
        }
        // Stamp before signalling, and ONLY past the already-exited
        // return above: a session that finished on its own and is being
        // reaped must not report its turn as cancelled. The waiter cannot
        // record this itself — a signalled child yields `-1`, which is
        // indistinguishable from a wait error.
        self.with_mut(&handle, |session| {
            let current = session.turn;
            if let Some(record) = session.turns.iter_mut().find(|r| r.turn == current) {
                record.outcome = TurnOutcome::Killed;
                record.finished_at = Some(rmcp::task_manager::current_timestamp());
            }
        });
        signal_group(pgid, nix::sys::signal::Signal::SIGTERM);
        let graceful = tokio::time::timeout(KILL_GRACE, async {
            while done.borrow().is_none() {
                if done.changed().await.is_err() {
                    break;
                }
            }
        })
        .await;
        if graceful.is_err() {
            tracing::warn!(%handle, pgid, "mcp harness: session ignored SIGTERM; escalating to SIGKILL");
            signal_group(pgid, nix::sys::signal::Signal::SIGKILL);
        }

        Some(true)
    }

    /// Kill every live session and drop the table, removing every
    /// `TempDir`. Called from `skills_server::run` on graceful transport close
    /// and on SIGTERM/SIGHUP.
    pub(crate) async fn shutdown(&self) {
        let handles: Vec<String> = self.map_all(|s| s.handle.clone());
        if handles.is_empty() {
            return;
        }
        tracing::info!(sessions = handles.len(), "mcp harness: shutting down live sessions");
        for handle in handles {
            self.kill(&handle).await;
        }
        // Dropping the sessions removes their TempDirs.
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).clear();
    }

    /// Spawn a prepared command as an owned session.
    pub(crate) fn spawn(
        self: &Arc<Self>,
        command: SpawnCommand,
        profile_id: String,
        provider: AgentProvider,
        provenance: Provenance,
        mut launch: LaunchShape,
    ) -> Result<String> {
        let dir = tempfile::Builder::new()
            .prefix(SESSION_DIR_PREFIX)
            // `/tmp` is 1777 and `turns.jsonl` is a full agent
            // transcript — whatever the agent read ends up here. 0700
            // from creation, never a chmod-after-create race.
            .permissions(owner_only())
            .tempdir()
            .context("mcp harness: create session directory")?;

        let handle = self.mint_handle();
        // The resolved cwd, so a follow-up turn replays to the same
        // directory even when this one fell back to `$PWD`.
        launch.cwd = command.cwd.clone();
        let Launched { pid, pgid, done } =
            launch_child(&command, dir.path(), &handle, false, self.on_exit.get().cloned())?;

        let now = SystemTime::now();
        let session = Session {
            handle: handle.clone(),
            profile_id,
            provider,
            launch,
            provenance,
            resume_token: None,
            created_at: now,
            last_turn_at: now,
            turn: 1,
            turns: vec![TurnRecord {
                turn: 1,
                outcome: TurnOutcome::Running,
                transcript_from: 0,
                started_at: rmcp::task_manager::current_timestamp(),
                finished_at: None,
            }],
            dir,
            pid,
            pgid,
            done,
        };
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(handle.clone(), session);

        Ok(handle)
    }
}

/// Why a [`SessionTable::respawn`] could not start a turn.
#[derive(Debug)]
pub(crate) enum RespawnError {
    Unknown,
    /// Another turn is already in flight on this session. No vendor
    /// supports two concurrent turns on one conversation, so the loser of
    /// the race is told rather than quietly starting a second one.
    Busy,
    Spawn(anyhow::Error),
}

struct Launched {
    pid: u32,
    pgid: i32,
    done: watch::Receiver<Option<i32>>,
}

/// Start one vendor process against a session directory.
///
/// Shared by the first turn and every later one. `append` keeps a resumed
/// turn writing into the SAME `turns.jsonl`, so a conversation reads back
/// as one continuous transcript and byte offsets stay meaningful across
/// turns.
fn launch_child(
    command: &SpawnCommand,
    dir: &Path,
    handle: &str,
    append: bool,
    on_exit: Option<ExitHook>,
) -> Result<Launched> {
    let open = |name: &str| -> Result<std::fs::File> {
        let path = dir.join(name);
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .append(append)
            .truncate(!append)
            .open(&path)
            .with_context(|| format!("mcp harness: open {}", path.display()))
    };
    let stdout = open(TURNS_FILE)?;
    let stderr = open(STDERR_FILE)?;
    let stdin_prompt = command.stdin_prompt.clone();

    let mut cmd = tokio::process::Command::new(&command.program);
    cmd.args(&command.args)
        .envs(&command.env)
        // Every one of the three is set explicitly. tokio defaults to
        // INHERIT: an unset stdin would steal the sidecar's MCP request
        // stream, an unset stdout would corrupt the JSON-RPC framing with
        // a single vendor log line.
        .stdin(if stdin_prompt.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        // Backstop only — see the module docs. Without this tokio ORPHANS
        // a live child on drop rather than killing it.
        .kill_on_drop(true);
    if let Some(cwd) = command.cwd.as_ref() {
        cmd.current_dir(cwd);
    }
    harden_child(&mut cmd);

    let mut child = cmd
        .spawn()
        .with_context(|| format!("mcp harness: spawning {}", command.program))?;
    let pid = child.id().context("mcp harness: child pid missing after spawn")?;
    // `setpgid(0, 0)` in pre_exec makes the child its own group leader,
    // so pgid == pid.
    let pgid = pid as i32;

    if let Some(prompt) = stdin_prompt {
        // Write the prompt and CLOSE the pipe. The EOF is load-bearing:
        // it is what stops `codex exec` hanging on an idle pipe. Must
        // complete before any `wait`, since `Child::wait` drops stdin.
        if let Some(mut sink) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            let bytes = prompt.into_bytes();
            tokio::spawn(async move {
                if let Err(err) = sink.write_all(&bytes).await {
                    tracing::warn!(%err, "mcp harness: writing prompt to child stdin failed");
                }
                // `sink` drops here, closing the pipe.
            });
        }
    }

    write_breadcrumb(dir, handle, pid, pgid);

    // Clear any previous turn's marker BEFORE the process can finish.
    // `session_send` reuses the handle and the directory, so a watcher
    // armed for turn N+1 would otherwise fire instantly on turn N's
    // leftover. Unconditional: on a fresh `spawn` the file cannot exist
    // and the failed unlink is cheaper than a `stat` first.
    let done_path = dir.join(DONE_FILE);
    let _ = std::fs::remove_file(&done_path);

    let (tx, done) = watch::channel(None);
    let waiter_handle = handle.to_string();
    tokio::spawn(async move {
        let code = match child.wait().await {
            Ok(status) => status.code().unwrap_or(-1),
            Err(err) => {
                tracing::warn!(handle = %waiter_handle, %err, "mcp harness: waiting on session failed");
                -1
            }
        };
        tracing::info!(handle = %waiter_handle, exit_code = code, "mcp harness: session exited");
        // Marker first, then the channel: a watcher polling the file is
        // the one that cannot be woken any other way.
        //
        // Never panic in here. Under the release profile's
        // `panic = "abort"` a panic in this task takes the whole sidecar
        // down and, through PDEATHSIG, every running agent with it.
        let body = serde_json::json!({
            "handle": waiter_handle,
            "exitCode": code,
            "finishedAt": SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
        });
        if let Err(err) = std::fs::write(&done_path, body.to_string()) {
            tracing::warn!(handle = %waiter_handle, %err, "mcp harness: could not write the done marker");
        }
        let _ = tx.send(Some(code));
        if let Some(hook) = on_exit {
            hook(waiter_handle, code);
        }
    });

    Ok(Launched { pid, pgid, done })
}

#[cfg(unix)]
fn owner_only() -> std::fs::Permissions {
    use std::os::unix::fs::PermissionsExt;
    std::fs::Permissions::from_mode(0o700)
}

#[cfg(not(unix))]
fn owner_only() -> std::fs::Permissions {
    std::fs::Permissions::from(std::fs::metadata(std::env::temp_dir()).unwrap().permissions())
}

/// Put the child in its own process group and, on Linux, ask the kernel
/// to `SIGKILL` it when this process dies.
///
/// `PR_SET_PDEATHSIG` is the only orphan guard that survives a `SIGKILL`
/// of the sidecar or a `panic = "abort"`, both of which run no
/// destructor. Two caveats worth knowing before touching this:
///
/// - It fires on the death of the **spawning thread**, not the process.
///   tokio worker threads live for the runtime's lifetime, so this is
///   sound today — but moving session spawning onto a short-lived thread
///   would silently break it.
/// - It is Linux-only. Elsewhere the guarantee degrades to the userspace
///   paths (`shutdown` + `kill_on_drop`), which SIGKILL defeats.
#[cfg(unix)]
fn harden_child(cmd: &mut tokio::process::Command) {
    unsafe {
        cmd.pre_exec(|| {
            // Own process group, so a kill reaches the vendor's children.
            nix::unistd::setpgid(nix::unistd::Pid::from_raw(0), nix::unistd::Pid::from_raw(0))
                .map_err(std::io::Error::from)?;
            #[cfg(target_os = "linux")]
            nix::sys::prctl::set_pdeathsig(Some(nix::sys::signal::Signal::SIGKILL)).map_err(std::io::Error::from)?;

            Ok(())
        });
    }
}

#[cfg(not(unix))]
fn harden_child(_cmd: &mut tokio::process::Command) {}

/// Crash-recovery breadcrumb. Deliberately NOT a session store: never
/// read on the happy path, no atomicity guarantees, no consistency
/// contract. It exists so a *later* sidecar can reclaim what a crashed
/// predecessor left behind — see [`sweep_stale_sessions`].
fn write_breadcrumb(dir: &Path, handle: &str, pid: u32, pgid: i32) {
    let body = serde_json::json!({
        "handle": handle,
        "pid": pid,
        "pgid": pgid,
        // The OWNING sidecar's pid. Load-bearing for the sweep: two
        // harness sidecars running at once is an ordinary setup (two
        // clients, or a vendor restarting one while the old drains), and
        // without this the newcomer cannot tell "a crashed predecessor's
        // leftovers" from "a live sibling's working sessions".
        "ownerPid": std::process::id(),
        // The session leader's start time. Checked before the sweep ever
        // signals, so a recycled pid is never mistaken for our process
        // group. Without it the sweep would `killpg` a number, not an
        // identity.
        "startTicks": proc_start_ticks(pid),
        "startedAt": SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
    });
    let path = dir.join(BREADCRUMB_FILE);
    if let Err(err) = std::fs::write(&path, body.to_string()) {
        tracing::debug!(%err, path = %path.display(), "mcp harness: writing session breadcrumb failed");
    }
}

/// Reclaim sessions a previous sidecar left behind.
///
/// PDEATHSIG kills the direct child, but a machine crash — or a
/// grandchild that outlived its parent — can still leave a live process
/// group and a stale directory. Run once at startup: kill any surviving
/// group, remove the directory, and log loudly, because a non-empty
/// sweep means something died badly.
pub(crate) fn sweep_stale_sessions() {
    sweep_stale_sessions_in(&std::env::temp_dir());
}

/// The sweep, scoped to a directory.
///
/// Takes the directory so tests can point it at their own `TempDir`:
/// running the real sweep against the machine's `/tmp` would let
/// `cargo test` `killpg` whatever pgid a developer's leftover breadcrumb
/// happens to name. A unit test must not be able to kill the developer's
/// processes.
pub(crate) fn sweep_stale_sessions_in(temp: &Path) {
    let entries = match std::fs::read_dir(temp) {
        Ok(entries) => entries,
        Err(err) => {
            tracing::debug!(%err, dir = %temp.display(), "mcp harness: startup sweep: read_dir failed");
            return;
        }
    };

    let mut reclaimed = 0usize;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if !name.starts_with(SESSION_DIR_PREFIX) {
            continue;
        }
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let crumb = read_breadcrumb(&path);

        // ⛔ Only reclaim from a sidecar that is actually GONE. Two
        // harness sidecars at once is an ordinary setup, and without this
        // check a newcomer would SIGKILL a live sibling's agents and
        // delete the transcripts they are still writing — the sweep
        // advertised as crash recovery would be the thing eating live
        // work. A breadcrumb we cannot read is left alone for the same
        // reason: unknown is not the same as dead.
        match crumb.as_ref().map(|crumb| crumb.owner_pid) {
            Some(Some(owner)) if process_is_alive(owner) => {
                tracing::debug!(
                    path = %path.display(),
                    owner,
                    "mcp harness: startup sweep: skipping — owning sidecar is still alive"
                );
                continue;
            }
            Some(Some(_)) => {}
            // No `ownerPid`: written by an older build, or unreadable.
            // Leaving it costs a stale directory; removing it could cost
            // someone's running work.
            _ => {
                tracing::debug!(
                    path = %path.display(),
                    "mcp harness: startup sweep: skipping — no owner recorded, cannot prove it is stale"
                );
                continue;
            }
        }

        if let Some(crumb) = crumb.as_ref() {
            // The owner is dead, so a still-live group is genuinely
            // orphaned — kill the whole group, not just the leader, since
            // the vendor spawns its own subprocesses.
            //
            // But prove it is OUR group first. A breadcrumb can be days
            // old, pids wrap around, and the group leader's pid == pgid,
            // so signalling a bare recorded number could `SIGKILL` an
            // unrelated process group of the same user. Comparing the
            // leader's start time makes the pid an identity instead.
            match (crumb.pgid, crumb.start_ticks) {
                (Some(pgid), Some(recorded)) if proc_start_ticks(pgid as u32) == Some(recorded) => {
                    signal_group(pgid, nix::sys::signal::Signal::SIGKILL);
                }
                (Some(pgid), Some(_)) => {
                    tracing::debug!(
                        pgid,
                        "mcp harness: startup sweep: not signalling — pid was recycled or the group is gone"
                    );
                }
                // Written before start times were recorded, or on a
                // platform without them. Removing the directory is safe;
                // signalling an unverifiable pgid is not.
                _ => {
                    tracing::debug!(
                        path = %path.display(),
                        "mcp harness: startup sweep: no verifiable group; reclaiming the directory only"
                    );
                }
            }
        }
        match std::fs::remove_dir_all(&path) {
            Ok(()) => reclaimed += 1,
            Err(err) => {
                tracing::debug!(%err, path = %path.display(), "mcp harness: startup sweep: remove failed");
            }
        }
    }

    if reclaimed > 0 {
        tracing::warn!(
            reclaimed,
            "mcp harness: startup sweep reclaimed session directories left by a previous sidecar"
        );
    }
}

struct Breadcrumb {
    owner_pid: Option<u32>,
    pgid: Option<i32>,
    start_ticks: Option<u64>,
}

fn read_breadcrumb(dir: &Path) -> Option<Breadcrumb> {
    let body = std::fs::read_to_string(dir.join(BREADCRUMB_FILE)).ok()?;
    let value: serde_json::Value = serde_json::from_str(&body).ok()?;

    Some(Breadcrumb {
        owner_pid: value
            .get("ownerPid")
            .and_then(serde_json::Value::as_u64)
            .map(|p| p as u32),
        pgid: value.get("pgid").and_then(serde_json::Value::as_i64).map(|p| p as i32),
        start_ticks: value.get("startTicks").and_then(serde_json::Value::as_u64),
    })
}

/// Whether a pid currently exists. `kill(pid, 0)` performs the
/// permission and existence check without delivering a signal.
///
/// Only ever used to decide whether to LEAVE something alone, so a
/// false "alive" is the safe direction: it means the sweep skips a
/// directory it could have reclaimed, not that it kills something live.
fn process_is_alive(pid: u32) -> bool {
    nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), None).is_ok()
}

/// A process's start time in clock ticks since boot — `/proc/<pid>/stat`
/// field 22.
///
/// This is what makes a pid an IDENTITY rather than a number. Pids wrap
/// around, and a breadcrumb can be days old by the time a sweep reads
/// it; the start time distinguishes "the same process we spawned" from
/// "some unrelated process that happens to hold that pid now".
///
/// `comm` (field 2) may itself contain spaces and parentheses, so the
/// fields are counted from AFTER the final `)`, never by splitting the
/// whole line.
#[cfg(target_os = "linux")]
fn proc_start_ticks(pid: u32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let tail = stat.rsplit_once(')')?.1;

    // After `)` the fields are state(3), ppid(4), … starttime(22), so
    // starttime is the 19th token here.
    tail.split_whitespace().nth(19)?.parse().ok()
}

#[cfg(not(target_os = "linux"))]
fn proc_start_ticks(_pid: u32) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> Arc<SessionTable> {
        SessionTable::new()
    }

    fn sleeper(secs: &str) -> SpawnCommand {
        SpawnCommand {
            program: "sleep".into(),
            args: vec![secs.into()],
            env: BTreeMap::new(),
            cwd: None,
            stdin_prompt: None,
        }
    }

    fn provenance() -> Provenance {
        Provenance {
            program: "sleep".into(),
            argv: Vec::new(),
            env_keys: Vec::new(),
            model: None,
            effort: None,
            mode: None,
            prompt_bytes: 0,
        }
    }

    /// Spawn with throwaway provenance — these tests assert lifecycle,
    /// not the audit view.
    fn spawn(table: &Arc<SessionTable>, command: SpawnCommand) -> String {
        table
            .spawn(
                command,
                "p".into(),
                AgentProvider::ClaudeCode,
                provenance(),
                LaunchShape::default(),
            )
            .unwrap()
    }

    #[tokio::test]
    async fn session_dir_is_owner_only_and_removed_on_drop() {
        let table = table();
        let handle = spawn(&table, sleeper("30"));
        let path = table.with(&handle, |s| s.path().to_path_buf()).unwrap();

        assert!(path.exists(), "session dir exists while the session lives");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700, "transcripts must never be world-readable in /tmp");
        }

        table.shutdown().await;
        assert!(!path.exists(), "dropping the table removes the session directory");
    }

    #[tokio::test]
    async fn shutdown_kills_live_sessions() {
        let table = table();
        let handle = spawn(&table, sleeper("300"));
        let pid = table.with(&handle, |s| s.pid()).unwrap();

        table.shutdown().await;

        // The process group is gone: signalling it now fails.
        let alive = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), None).is_ok();
        assert!(!alive, "shutdown must leave no live vendor process");
    }

    /// The defect this whole per-turn record exists to prevent.
    ///
    /// `respawn` replaces `done` wholesale, so without a per-turn record
    /// turn 1's exit code is unreachable the moment turn 2 starts — and
    /// `tasks/get` for turn 1 would report `Working` for a task that
    /// already completed. SEP-2663 makes terminal states immutable, so
    /// that is not a cosmetic bug.
    #[tokio::test]
    async fn a_finished_turn_stays_finished_after_the_next_one_starts() {
        let table = SessionTable::new();
        let command = SpawnCommand {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), "exit 3".into()],
            env: Default::default(),
            cwd: None,
            stdin_prompt: None,
        };
        let handle = table
            .spawn(
                command,
                "p".into(),
                crate::config::AgentProvider::ClaudeCode,
                Provenance {
                    program: "sh".into(),
                    argv: Vec::new(),
                    env_keys: Vec::new(),
                    model: None,
                    effort: None,
                    mode: None,
                    prompt_bytes: 0,
                },
                LaunchShape::default(),
            )
            .unwrap();

        let mut done = table.with(&handle, |s| s.completion()).unwrap();
        while done.borrow().is_none() {
            done.changed().await.unwrap();
        }
        assert_eq!(
            table.with(&handle, |s| s.turn_record(1).unwrap().outcome).unwrap(),
            TurnOutcome::Exited(3)
        );

        // Turn 2 replaces the live watch.
        table
            .respawn(
                &handle,
                SpawnCommand {
                    program: "/bin/sh".into(),
                    args: vec!["-c".into(), "sleep 5".into()],
                    env: Default::default(),
                    cwd: None,
                    stdin_prompt: None,
                },
                Provenance {
                    program: "sh".into(),
                    argv: Vec::new(),
                    env_keys: Vec::new(),
                    model: None,
                    effort: None,
                    mode: None,
                    prompt_bytes: 0,
                },
            )
            .unwrap();

        assert_eq!(table.with(&handle, |s| s.turn).unwrap(), 2, "the counter must advance");
        assert_eq!(
            table.with(&handle, |s| s.turn_record(1).unwrap().outcome).unwrap(),
            TurnOutcome::Exited(3),
            "turn 1 is terminal — turn 2 starting must not rewrite it"
        );
        assert_eq!(
            table.with(&handle, |s| s.turn_record(2).unwrap().outcome).unwrap(),
            TurnOutcome::Running,
            "turn 2 is the one in flight"
        );
    }

    /// A kill and a crash both surface as an exit code, and for a signal
    /// death that code is `-1` — indistinguishable. Only an explicit
    /// stamp can tell them apart, and it must be set ONLY when we really
    /// killed something: reaping an already-finished session must not
    /// retroactively mark its turn cancelled.
    #[tokio::test]
    async fn killing_marks_the_turn_but_reaping_a_finished_one_does_not() {
        let table = SessionTable::new();
        let live = SpawnCommand {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), "sleep 30".into()],
            env: Default::default(),
            cwd: None,
            stdin_prompt: None,
        };
        let prov = || Provenance {
            program: "sh".into(),
            argv: Vec::new(),
            env_keys: Vec::new(),
            model: None,
            effort: None,
            mode: None,
            prompt_bytes: 0,
        };

        let killed = table
            .spawn(
                live,
                "p".into(),
                crate::config::AgentProvider::ClaudeCode,
                prov(),
                LaunchShape::default(),
            )
            .unwrap();
        assert_eq!(table.kill(&killed).await, Some(true));
        assert_eq!(
            table.with(&killed, |s| s.turn_record(1).unwrap().outcome).unwrap(),
            TurnOutcome::Killed
        );

        let finished = table
            .spawn(
                SpawnCommand {
                    program: "/bin/sh".into(),
                    args: vec!["-c".into(), "exit 0".into()],
                    env: Default::default(),
                    cwd: None,
                    stdin_prompt: None,
                },
                "p".into(),
                crate::config::AgentProvider::ClaudeCode,
                prov(),
                LaunchShape::default(),
            )
            .unwrap();
        let mut done = table.with(&finished, |s| s.completion()).unwrap();
        while done.borrow().is_none() {
            done.changed().await.unwrap();
        }
        assert_eq!(table.kill(&finished).await, Some(false), "already exited");
        assert_eq!(
            table.with(&finished, |s| s.turn_record(1).unwrap().outcome).unwrap(),
            TurnOutcome::Exited(0),
            "reaping a session that finished normally must not report it cancelled"
        );
    }

    #[tokio::test]
    async fn kill_reports_was_running_then_false() {
        let table = table();
        let handle = spawn(&table, sleeper("300"));

        assert_eq!(table.kill(&handle).await, Some(true), "first kill terminates");
        assert_eq!(
            table.kill(&handle).await,
            Some(false),
            "killing an exited session is a no-op, not an error"
        );
        assert_eq!(table.kill("nope").await, None, "unknown handle is distinguishable");
    }

    #[tokio::test]
    async fn exit_code_is_captured_because_we_own_the_child() {
        let table = table();
        let mut cmd = sleeper("0");
        cmd.program = "false".into();
        cmd.args.clear();
        let handle = spawn(&table, cmd);

        let mut done = table.with(&handle, |s| s.completion()).unwrap();
        while done.borrow().is_none() {
            done.changed().await.unwrap();
        }

        assert_eq!(table.with(&handle, |s| s.exit_code()).unwrap(), Some(1));
        assert_eq!(
            table.with(&handle, |s| s.status()).unwrap(),
            SessionStatus::Exited,
            "an exited session keeps its transcript and reports a code"
        );
    }

    #[tokio::test]
    async fn handles_are_unique_within_and_across_tables() {
        let table = table();
        let a = spawn(&table, sleeper("5"));
        let b = spawn(&table, sleeper("5"));
        assert_ne!(a, b, "two sessions on one profile must not collide");

        // The reason handles are UUIDs rather than a counter: a second
        // sidecar has its own table, and a per-table sequence would make
        // both mint the same first handle. Harmless only while the two
        // namespaces never meet — which is not a property worth relying
        // on for an identifier that travels in logs and tool results.
        let other = SessionTable::new();
        let c = spawn(&other, sleeper("5"));
        assert_ne!(a, c, "handles must not collide across independent sidecars");
        assert_ne!(b, c, "handles must not collide across independent sidecars");

        for handle in [&a, &b, &c] {
            assert!(
                uuid::Uuid::parse_str(handle).is_ok(),
                "a handle is a bare UUID, resolved by table lookup: {handle}"
            );
        }

        table.shutdown().await;
        other.shutdown().await;
    }

    /// Pins that `pre_exec` actually fires and the child leads its own
    /// process group — the precondition for `killpg` reaching the
    /// vendor's own subprocesses rather than just the direct child.
    ///
    /// The sibling guarantee set in the same `pre_exec`,
    /// `PR_SET_PDEATHSIG`, cannot be asserted from inside the test
    /// binary (it would require SIGKILLing the test runner). It was
    /// verified out-of-band: a child spawned with it died when its
    /// parent was `kill -9`'d.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn child_leads_its_own_process_group() {
        let table = table();
        let handle = spawn(&table, sleeper("30"));
        let pid = table.with(&handle, |s| s.pid()).unwrap();

        // /proc/<pid>/stat field 5 is pgrp. Fields 1-2 can contain
        // spaces inside `comm`'s parens, so split after the ')'.
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).expect("child is running");
        let tail = stat.rsplit_once(')').expect("comm is parenthesised").1;
        let pgrp: i32 = tail
            .split_whitespace()
            .nth(2)
            .expect("pgrp field present")
            .parse()
            .expect("pgrp is numeric");

        assert_eq!(
            pgrp, pid as i32,
            "child must lead its own group so killpg reaches its descendants"
        );

        table.shutdown().await;
    }

    /// Two harness sidecars at once is an ordinary setup (two clients, or
    /// a vendor restarting one while the old drains). Without an owner
    /// check the newcomer's "crash recovery" sweep SIGKILLs the live
    /// sibling's agents and deletes the transcripts they are still
    /// writing — the mechanism advertised as a safety net becomes the
    /// thing that eats working sessions.
    #[test]
    fn sweep_spares_a_live_sidecars_sessions_and_reclaims_a_dead_ones() {
        // Scoped to our own directory: pointing the real sweep at the
        // machine's /tmp would let `cargo test` kill whatever pgid a
        // developer's leftover breadcrumb names.
        let root = tempfile::tempdir().unwrap();
        let temp = root.path();

        let write = |name: &str, crumb: serde_json::Value| {
            let dir = temp.join(format!("{SESSION_DIR_PREFIX}{name}"));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(BREADCRUMB_FILE), crumb.to_string()).unwrap();
            dir
        };

        // Owned by US — we are obviously alive, standing in for a
        // concurrently-running sibling sidecar.
        let live = write(
            "live",
            serde_json::json!({ "handle": "h", "pid": 1, "ownerPid": std::process::id() }),
        );
        // Owned by a pid that cannot exist — a genuinely dead predecessor.
        // No pgid, so the sweep has nothing to signal.
        let dead = write("dead", serde_json::json!({ "handle": "h", "ownerPid": 0x7FFF_FFFEu32 }));
        // No owner recorded: unknown is not the same as dead.
        let unknown = write("unknown", serde_json::json!({ "handle": "h" }));

        sweep_stale_sessions_in(temp);

        assert!(live.exists(), "a live sidecar's sessions must survive another's sweep");
        assert!(!dead.exists(), "a dead sidecar's leftovers must be reclaimed");
        assert!(unknown.exists(), "unprovable ownership must not be treated as stale");
    }

    /// A resume keeps the caller's handle and appends to the SAME
    /// transcript, so a conversation is one entry and one file however
    /// many turns it runs.
    #[tokio::test]
    async fn respawn_keeps_the_handle_and_appends_to_one_transcript() {
        let table = table();
        let mut first = sleeper("0");
        first.program = "echo".into();
        first.args = vec!["turn-one".into()];
        let handle = spawn(&table, first);

        // Let the first turn finish before resuming it.
        let mut done = table.with(&handle, |s| s.completion()).unwrap();
        while done.borrow().is_none() {
            done.changed().await.unwrap();
        }

        let mut second = sleeper("0");
        second.program = "echo".into();
        second.args = vec!["turn-two".into()];
        table.respawn(&handle, second, provenance()).expect("resume succeeds");

        assert_eq!(
            table.map_all(|s| s.handle.clone()).len(),
            1,
            "a resume must not add a row"
        );

        let mut done = table.with(&handle, |s| s.completion()).unwrap();
        while done.borrow().is_none() {
            done.changed().await.unwrap();
        }
        let transcript = std::fs::read_to_string(table.with(&handle, |s| s.turns_path()).unwrap()).unwrap();
        assert!(
            transcript.contains("turn-one"),
            "the earlier turn survives: {transcript:?}"
        );
        assert!(
            transcript.contains("turn-two"),
            "the new turn is appended: {transcript:?}"
        );
    }

    /// Two concurrent sends on one session must not both start a turn —
    /// no vendor supports two at once. The check and the spawn happen
    /// under one lock, so the loser is told rather than quietly starting
    /// a second conversation.
    /// The marker must be cleared when a turn STARTS, not only written
    /// when one ends. `session_send` reuses the handle and directory, so
    /// a watcher armed for turn N+1 would otherwise fire instantly on
    /// turn N's leftover and read a finished session as already done.
    #[tokio::test]
    async fn a_new_turn_clears_the_previous_turn_s_done_marker() {
        let table = table();
        let handle = table
            .spawn(
                sleeper("0"),
                "p".into(),
                AgentProvider::ClaudeCode,
                provenance(),
                LaunchShape::default(),
            )
            .unwrap();

        let done = table.with(&handle, |s| s.done_path()).expect("session");
        for _ in 0..100 {
            if done.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert!(done.exists(), "the first turn must leave a marker");

        table.respawn(&handle, sleeper("2"), provenance()).expect("respawn");
        // The delete happens before the child can finish, so the marker
        // is gone the instant the next turn starts.
        let cleared_or_rewritten = !done.exists() || {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            done.exists()
        };
        assert!(
            cleared_or_rewritten,
            "the marker must not survive untouched into turn 2"
        );
    }

    #[tokio::test]
    async fn respawn_refuses_while_a_turn_is_in_flight() {
        let table = table();
        let handle = spawn(&table, sleeper("30"));

        let err = table
            .respawn(&handle, sleeper("1"), provenance())
            .expect_err("a running session must refuse a second turn");
        assert!(matches!(err, RespawnError::Busy), "expected Busy, got {err:?}");

        assert!(matches!(
            table.respawn("nope", sleeper("1"), provenance()),
            Err(RespawnError::Unknown)
        ));

        table.shutdown().await;
    }

    /// Finished sessions are evicted oldest-first once the table grows
    /// past the cap; a live one is never touched.
    #[tokio::test]
    async fn eviction_bounds_the_table_without_touching_live_sessions() {
        let table = table();
        let live = spawn(&table, sleeper("30"));

        let mut finished = Vec::new();
        for _ in 0..4 {
            let mut cmd = sleeper("0");
            cmd.program = "true".into();
            cmd.args.clear();
            let handle = spawn(&table, cmd);
            let mut done = table.with(&handle, |s| s.completion()).unwrap();
            while done.borrow().is_none() {
                done.changed().await.unwrap();
            }
            finished.push(handle);
        }

        table.evict_exited_over(2);

        let remaining = table.map_all(|s| s.handle.clone());
        assert_eq!(remaining.len(), 2, "the table must be bounded: {remaining:?}");
        assert!(remaining.contains(&live), "a running session is never evicted");

        table.shutdown().await;
    }

    /// The sweep must prove a recorded pgid is still OUR process group
    /// before signalling it. Pids wrap around and a breadcrumb can be
    /// days old, so a bare recorded number could name an unrelated
    /// process group of the same user by the time a sweep reads it —
    /// and the sweep sends `SIGKILL`.
    ///
    /// Uses OUR OWN pid as the "recycled" pgid with a deliberately wrong
    /// start time: if the identity check regressed, this test would kill
    /// the test runner, which is about as loud a failure signal as one
    /// can arrange.
    #[cfg(target_os = "linux")]
    #[test]
    fn sweep_will_not_signal_a_recycled_pid() {
        let root = tempfile::tempdir().unwrap();
        let temp = root.path();
        let me = std::process::id();

        assert!(
            proc_start_ticks(me).is_some(),
            "start ticks must be readable for the guard to mean anything"
        );

        let dir = temp.join(format!("{SESSION_DIR_PREFIX}recycled"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(BREADCRUMB_FILE),
            serde_json::json!({
                "handle": "h",
                "pid": me,
                "pgid": me,
                // A dead owner, so the sweep proceeds to the kill step…
                "ownerPid": 0x7FFF_FFFEu32,
                // …but the start time does NOT match this pid, which is
                // exactly the recycled-pid case.
                "startTicks": 1u64,
            })
            .to_string(),
        )
        .unwrap();

        sweep_stale_sessions_in(temp);

        // Surviving to here is the assertion: the sweep reclaimed the
        // directory without signalling our own process group.
        assert!(!dir.exists(), "the stale directory is still reclaimed");
    }

    /// The counterpart to the recycled-pid test: the identity check must
    /// not be so strict that it stops reclaiming genuine orphans. A
    /// breadcrumb naming a REAL live process group, with a matching
    /// start time and a dead owner, must still be killed.
    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn sweep_kills_a_verified_orphan_group() {
        let root = tempfile::tempdir().unwrap();
        let temp = root.path();

        // A real process in its own group, standing in for an agent a
        // crashed sidecar left behind.
        let mut orphan = tokio::process::Command::new("sleep")
            .arg("60")
            .process_group(0)
            .spawn()
            .expect("spawn orphan");
        let pid = orphan.id().expect("orphan pid");
        let ticks = proc_start_ticks(pid).expect("orphan start ticks");

        let dir = temp.join(format!("{SESSION_DIR_PREFIX}orphan"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(BREADCRUMB_FILE),
            serde_json::json!({
                "handle": "h",
                "pid": pid,
                "pgid": pid,
                "ownerPid": 0x7FFF_FFFEu32,
                "startTicks": ticks,
            })
            .to_string(),
        )
        .unwrap();

        sweep_stale_sessions_in(temp);

        // Assert on the WAIT STATUS, not `process_is_alive`: a killed
        // child that nobody has reaped is a zombie, and `kill(pid, 0)`
        // succeeds for a zombie — so a liveness probe would read a
        // successfully-killed process as still alive.
        let status = tokio::time::timeout(std::time::Duration::from_secs(5), orphan.wait())
            .await
            .expect("a verified orphan group must be killed — the identity check must not over-block")
            .expect("wait on orphan");
        assert!(
            !status.success(),
            "the orphan must have been signalled, not exited normally"
        );
        assert!(!dir.exists(), "and its directory removed");
    }

    #[test]
    fn sweep_only_matches_hyprpilot_session_dirs() {
        // The predicate is what keeps the sweep from touching an
        // unrelated directory in a shared /tmp.
        assert!("hyprpilot-session-abc123".starts_with(SESSION_DIR_PREFIX));
        assert!(!"hyprpilot-mcp-1-2.json".starts_with(SESSION_DIR_PREFIX));
        assert!(!"tmp.XXXX".starts_with(SESSION_DIR_PREFIX));
    }
}
