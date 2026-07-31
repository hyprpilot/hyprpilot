//! Harness session store — the sidecar owns every agent it spawns.
//!
//! A session is a **direct child** of `hyprpilot mcp serve`, waited on
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

/// One live or finished agent session.
#[derive(Debug)]
pub(crate) struct Session {
    pub handle: String,
    pub profile_id: String,
    pub provider: AgentProvider,
    pub cwd: Option<PathBuf>,
    pub provenance: Provenance,
    /// The vendor's own session id, parsed out of the first turn's event
    /// stream. `None` until the vendor emits it (or forever, if the turn
    /// failed before it did) — which is why `session_send` reports a clean
    /// error instead of assuming one exists.
    pub vendor_session_id: Option<String>,
    pub created_at: SystemTime,
    pub last_turn_at: SystemTime,
    /// Removed from disk when this struct drops.
    dir: TempDir,
    pid: u32,
    pgid: i32,
    /// Exit code once the child has been reaped. Watch rather than a
    /// plain field so `wait: true` can await completion without holding
    /// the table lock across an await.
    done: watch::Receiver<Option<i32>>,
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

/// The in-process session table. Bounded by the sidecar's own lifetime —
/// there is no persistence and no cross-launch state.
#[derive(Debug, Default)]
pub(crate) struct SessionTable {
    inner: Mutex<BTreeMap<String, Session>>,
    /// Monotonic counter feeding handle minting, so two sessions spawned
    /// in the same nanosecond still get distinct handles.
    seq: Mutex<u64>,
}

impl SessionTable {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn mint_handle(&self, profile_id: &str, provider: AgentProvider) -> String {
        let mut seq = self.seq.lock().unwrap_or_else(|p| p.into_inner());
        *seq += 1;
        let slug: String = profile_id
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();

        format!("hp-{}-{}-{}", provider.wire_id(), slug, seq)
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
    /// `TempDir`. Called from `serve::run` on graceful transport close
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
    ) -> Result<String> {
        let dir = tempfile::Builder::new()
            .prefix(SESSION_DIR_PREFIX)
            // `/tmp` is 1777 and `turns.jsonl` is a full agent
            // transcript — whatever the agent read ends up here. 0700
            // from creation, never a chmod-after-create race.
            .permissions(owner_only())
            .tempdir()
            .context("mcp harness: create session directory")?;

        let stdout = std::fs::File::create(dir.path().join(TURNS_FILE))
            .with_context(|| format!("mcp harness: create {TURNS_FILE}"))?;
        let stderr = std::fs::File::create(dir.path().join(STDERR_FILE))
            .with_context(|| format!("mcp harness: create {STDERR_FILE}"))?;

        let handle = self.mint_handle(&profile_id, provider);
        let cwd = command.cwd.clone();
        let stdin_prompt = command.stdin_prompt.clone();

        let mut cmd = tokio::process::Command::new(&command.program);
        cmd.args(&command.args)
            .envs(&command.env)
            // Every one of the three is set explicitly. tokio defaults to
            // INHERIT: an unset stdin would steal the sidecar's MCP
            // request stream, an unset stdout would corrupt the
            // JSON-RPC framing with a single vendor log line.
            .stdin(if stdin_prompt.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            // Backstop only — see the module docs. Without this tokio
            // ORPHANS a live child on drop rather than killing it.
            .kill_on_drop(true);
        if let Some(cwd) = command.cwd.as_ref() {
            cmd.current_dir(cwd);
        }
        harden_child(&mut cmd);

        let mut child = cmd
            .spawn()
            .with_context(|| format!("mcp harness: spawning {}", command.program))?;
        let pid = child.id().context("mcp harness: child pid missing after spawn")?;
        // `setpgid(0, 0)` in pre_exec makes the child its own group
        // leader, so pgid == pid.
        let pgid = pid as i32;

        if let Some(prompt) = stdin_prompt {
            // Write the prompt and CLOSE the pipe. The EOF is
            // load-bearing: it is what stops `codex exec` hanging on an
            // idle pipe. Must complete before any `wait`, since
            // `Child::wait` drops stdin itself.
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

        write_breadcrumb(dir.path(), &handle, pid, pgid);

        let (tx, done) = watch::channel(None);
        let waiter_handle = handle.clone();
        tokio::spawn(async move {
            let code = match child.wait().await {
                Ok(status) => status.code().unwrap_or(-1),
                Err(err) => {
                    tracing::warn!(handle = %waiter_handle, %err, "mcp harness: waiting on session failed");
                    -1
                }
            };
            tracing::info!(handle = %waiter_handle, exit_code = code, "mcp harness: session exited");
            let _ = tx.send(Some(code));
        });

        let now = SystemTime::now();
        let session = Session {
            handle: handle.clone(),
            profile_id,
            provider,
            cwd,
            provenance,
            vendor_session_id: None,
            created_at: now,
            last_turn_at: now,
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
    let temp = std::env::temp_dir();
    let entries = match std::fs::read_dir(&temp) {
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

        if let Some(pgid) = crumb.as_ref().and_then(|crumb| crumb.pgid) {
            // The owner is dead, so a still-live group is genuinely
            // orphaned — kill the whole group, not just the leader, since
            // the vendor spawns its own subprocesses.
            signal_group(pgid, nix::sys::signal::Signal::SIGKILL);
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
            .spawn(command, "p".into(), AgentProvider::ClaudeCode, provenance())
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
    async fn handles_are_unique_per_session() {
        let table = table();
        let a = spawn(&table, sleeper("5"));
        let b = spawn(&table, sleeper("5"));

        assert_ne!(a, b, "two sessions on one profile must not collide");
        table.shutdown().await;
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
        let temp = std::env::temp_dir();

        // Owned by US — we are obviously alive, standing in for a
        // concurrently-running sibling sidecar.
        let live = temp.join(format!("{SESSION_DIR_PREFIX}livetest-{}", std::process::id()));
        std::fs::create_dir_all(&live).unwrap();
        std::fs::write(
            live.join(BREADCRUMB_FILE),
            serde_json::json!({ "handle": "h", "pid": 1, "pgid": 0, "ownerPid": std::process::id() }).to_string(),
        )
        .unwrap();

        // Owned by a pid that cannot exist — a genuinely dead predecessor.
        let dead = temp.join(format!("{SESSION_DIR_PREFIX}deadtest-{}", std::process::id()));
        std::fs::create_dir_all(&dead).unwrap();
        std::fs::write(
            dead.join(BREADCRUMB_FILE),
            // pgid 0 would signal OUR OWN group, so leave it out entirely.
            serde_json::json!({ "handle": "h", "pid": 999, "ownerPid": 0x7FFF_FFFEu32 }).to_string(),
        )
        .unwrap();

        // No owner recorded: unknown is not the same as dead, so it must
        // be left alone rather than assumed reclaimable.
        let unknown = temp.join(format!("{SESSION_DIR_PREFIX}unknowntest-{}", std::process::id()));
        std::fs::create_dir_all(&unknown).unwrap();
        std::fs::write(unknown.join(BREADCRUMB_FILE), "{\"handle\":\"h\"}").unwrap();

        sweep_stale_sessions();

        assert!(live.exists(), "a live sidecar's sessions must survive another's sweep");
        assert!(!dead.exists(), "a dead sidecar's leftovers must be reclaimed");
        assert!(unknown.exists(), "unprovable ownership must not be treated as stale");

        let _ = std::fs::remove_dir_all(&live);
        let _ = std::fs::remove_dir_all(&unknown);
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
