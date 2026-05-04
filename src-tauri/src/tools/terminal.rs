//! Terminal primitives used by the ACP adapter.
//!
//! `Terminals` owns the live registry of spawned child processes keyed
//! by `(session_key, TerminalId)`. Session identity is an opaque
//! string at this boundary — the ACP adapter stringifies `SessionId`
//! before dispatching. Errors are a domain type (`TerminalError`);
//! ACP mapping stays in the adapter.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Instant;

use agent_client_protocol::schema::{
    CreateTerminalRequest, CreateTerminalResponse, KillTerminalRequest, KillTerminalResponse, ReleaseTerminalRequest,
    ReleaseTerminalResponse, TerminalExitStatus, TerminalId, TerminalOutputRequest, TerminalOutputResponse,
    WaitForTerminalExitRequest, WaitForTerminalExitResponse,
};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};
use tokio::sync::{broadcast, Mutex};

use super::sandbox::{Sandbox, SandboxError};

/// 1 MiB is generous for interactive output while bounding daemon
/// memory against a runaway child when the agent doesn't set a limit.
const DEFAULT_OUTPUT_LIMIT: u64 = 1024 * 1024;

/// Fixed-size chunk per `read` syscall. 4 KiB matches tokio's default
/// pipe buffer and avoids the `read_until(b'\n')` stall on binary
/// output or long unterminated lines.
const READ_CHUNK: usize = 4096;

/// Broadcast capacity for terminal output events. Slow consumers
/// (lagged UI, ctl subscribers) drop messages — buffer is per-tick,
/// not the source-of-truth (`OutputBuffer` keeps the full snapshot).
const TERMINAL_EVENT_CAPACITY: usize = 256;

type RegistryKey = (String, TerminalId);

/// Per-terminal stream / exit event published by [`Terminals`] when
/// it reads child stdout / stderr or sees the child exit. The ACP
/// adapter subscribes via [`Terminals::subscribe`] and bridges each
/// payload onto the generic `InstanceEvent::Terminal`.
#[derive(Debug, Clone)]
pub struct TerminalToolEvent {
    pub session_key: String,
    pub terminal_id: String,
    pub kind: TerminalToolEventKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TerminalToolEventKind {
    Output {
        stream: TerminalToolStream,
        data: String,
    },
    Exit {
        exit_code: Option<i32>,
        signal: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TerminalToolStream {
    Stdout,
    Stderr,
}

#[derive(Debug, thiserror::Error)]
pub enum TerminalError {
    #[error(transparent)]
    Sandbox(#[from] SandboxError),
    #[error("unknown terminal id: {0}")]
    UnknownTerminal(String),
    #[error("terminal exit status unavailable")]
    ExitStatusUnavailable,
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug)]
pub struct TerminalState {
    pub child: Option<Child>,
    pub buffer: Arc<Mutex<OutputBuffer>>,
    pub exit: Arc<Mutex<Option<TerminalExitStatus>>>,
    /// Captured at spawn time for the exit-log duration field.
    pub started_at: Instant,
}

/// Ring-style buffer: grows until `limit`, then shifts bytes off the
/// front on each append. Drops whole UTF-8 code points when trimming so
/// the retained view is always valid `str` — matches the crate's
/// `output_byte_limit` contract ("MUST truncate at a character
/// boundary").
#[derive(Debug, Default)]
pub struct OutputBuffer {
    data: Vec<u8>,
    truncated: bool,
}

impl OutputBuffer {
    pub fn push(&mut self, bytes: &[u8], limit: u64) {
        self.data.extend_from_slice(bytes);
        let limit = limit as usize;
        if self.data.len() > limit {
            self.truncated = true;
            let excess = self.data.len() - limit;
            let mut drop = excess;
            while drop < self.data.len() && (self.data[drop] & 0b1100_0000) == 0b1000_0000 {
                drop += 1;
            }
            self.data.drain(..drop);
        }
    }

    pub fn snapshot(&self) -> (String, bool) {
        (String::from_utf8_lossy(&self.data).into_owned(), self.truncated)
    }
}

#[derive(Debug)]
pub struct Terminals {
    sandbox: Sandbox,
    registry: Arc<Mutex<HashMap<RegistryKey, TerminalState>>>,
    /// Live-output broadcast. Subscribers receive every stdout / stderr
    /// chunk + exit; lagged subscribers drop messages silently — the
    /// `OutputBuffer` is the source of truth for the full snapshot.
    events_tx: broadcast::Sender<TerminalToolEvent>,
}

impl Terminals {
    pub fn new(sandbox: Sandbox) -> Self {
        let (events_tx, _) = broadcast::channel(TERMINAL_EVENT_CAPACITY);
        Self {
            sandbox,
            registry: Arc::new(Mutex::new(HashMap::new())),
            events_tx,
        }
    }

    /// Subscribe to live terminal events. Consumers must handle
    /// `broadcast::error::RecvError::Lagged` — the channel silently
    /// drops messages otherwise.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<TerminalToolEvent> {
        self.events_tx.subscribe()
    }

    pub async fn create(
        &self,
        session_key: impl AsRef<str>,
        req: CreateTerminalRequest,
    ) -> Result<CreateTerminalResponse, TerminalError> {
        let cwd = match &req.cwd {
            Some(p) => self.sandbox.resolve(p)?,
            None => self.sandbox.root().to_path_buf(),
        };
        let limit = req.output_byte_limit.unwrap_or(DEFAULT_OUTPUT_LIMIT);
        let terminal_id = TerminalId::new(uuid::Uuid::new_v4().to_string());

        tracing::debug!(
            session = session_key.as_ref(),
            terminal = %terminal_id.0,
            cwd = %cwd.display(),
            command = %req.command,
            args_count = req.args.len(),
            "tools::terminal: create"
        );

        let mut cmd = Command::new(&req.command);
        cmd.args(&req.args)
            .current_dir(&cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for var in &req.env {
            cmd.env(&var.name, &var.value);
        }

        let mut child = cmd.spawn()?;
        let buffer = Arc::new(Mutex::new(OutputBuffer::default()));
        let exit = Arc::new(Mutex::new(None::<TerminalExitStatus>));

        let session_key_str = session_key.as_ref().to_string();
        let terminal_id_str = terminal_id.0.to_string();
        if let Some(stdout) = child.stdout.take() {
            spawn_buffer_reader(
                stdout,
                BufferReaderConfig {
                    buffer: buffer.clone(),
                    limit,
                    events_tx: self.events_tx.clone(),
                    session_key: session_key_str.clone(),
                    terminal_id: terminal_id_str.clone(),
                    stream: TerminalToolStream::Stdout,
                },
            );
        }
        if let Some(stderr) = child.stderr.take() {
            spawn_buffer_reader(
                stderr,
                BufferReaderConfig {
                    buffer: buffer.clone(),
                    limit,
                    events_tx: self.events_tx.clone(),
                    session_key: session_key_str.clone(),
                    terminal_id: terminal_id_str.clone(),
                    stream: TerminalToolStream::Stderr,
                },
            );
        }

        let state = TerminalState {
            child: Some(child),
            buffer,
            exit,
            started_at: Instant::now(),
        };

        let mut registry = self.registry.lock().await;
        registry.insert((session_key.as_ref().to_string(), terminal_id.clone()), state);
        Ok(CreateTerminalResponse::new(terminal_id))
    }

    pub async fn output(
        &self,
        session_key: impl AsRef<str>,
        req: TerminalOutputRequest,
    ) -> Result<TerminalOutputResponse, TerminalError> {
        let key = (session_key.as_ref().to_string(), req.terminal_id.clone());
        let registry = self.registry.lock().await;
        let state = registry
            .get(&key)
            .ok_or_else(|| TerminalError::UnknownTerminal(req.terminal_id.0.to_string()))?;
        let (output, truncated) = state.buffer.lock().await.snapshot();
        let exit_status = state.exit.lock().await.clone();
        let mut resp = TerminalOutputResponse::new(output, truncated);
        resp.exit_status = exit_status;
        Ok(resp)
    }

    pub async fn wait(
        &self,
        session_key: impl AsRef<str>,
        req: WaitForTerminalExitRequest,
    ) -> Result<WaitForTerminalExitResponse, TerminalError> {
        let key = (session_key.as_ref().to_string(), req.terminal_id.clone());
        let (child, exit_slot, started_at) = {
            let mut registry = self.registry.lock().await;
            let state = registry
                .get_mut(&key)
                .ok_or_else(|| TerminalError::UnknownTerminal(req.terminal_id.0.to_string()))?;
            (state.child.take(), state.exit.clone(), state.started_at)
        };

        let status = match child {
            Some(mut child) => {
                let out = child.wait().await?;
                let status = exit_status_from(&out);
                let duration_ms = started_at.elapsed().as_millis();
                tracing::debug!(
                    session = session_key.as_ref(),
                    terminal = %req.terminal_id.0,
                    exit_code = ?status.exit_code,
                    signal = ?status.signal,
                    duration_ms = %duration_ms,
                    "tools::terminal: exit"
                );
                *exit_slot.lock().await = Some(status.clone());
                let _ = self.events_tx.send(TerminalToolEvent {
                    session_key: session_key.as_ref().to_string(),
                    terminal_id: req.terminal_id.0.to_string(),
                    kind: TerminalToolEventKind::Exit {
                        exit_code: status.exit_code.map(|c| c as i32),
                        signal: status.signal.clone(),
                    },
                });
                status
            }
            None => exit_slot
                .lock()
                .await
                .clone()
                .ok_or(TerminalError::ExitStatusUnavailable)?,
        };
        Ok(WaitForTerminalExitResponse::new(status))
    }

    pub async fn kill(
        &self,
        session_key: impl AsRef<str>,
        req: KillTerminalRequest,
    ) -> Result<KillTerminalResponse, TerminalError> {
        let key = (session_key.as_ref().to_string(), req.terminal_id.clone());
        let mut registry = self.registry.lock().await;
        let state = registry
            .get_mut(&key)
            .ok_or_else(|| TerminalError::UnknownTerminal(req.terminal_id.0.to_string()))?;
        if let Some(child) = state.child.as_mut() {
            let _ = child.start_kill();
        }
        Ok(KillTerminalResponse::new())
    }

    pub async fn release(
        &self,
        session_key: impl AsRef<str>,
        req: ReleaseTerminalRequest,
    ) -> Result<ReleaseTerminalResponse, TerminalError> {
        let key = (session_key.as_ref().to_string(), req.terminal_id.clone());
        let removed = {
            let mut registry = self.registry.lock().await;
            registry.remove(&key)
        };
        if let Some(mut state) = removed {
            if let Some(mut child) = state.child.take() {
                let _ = child.start_kill();
                let _ = child.wait().await;
            }
        }
        Ok(ReleaseTerminalResponse::new())
    }

    /// Release every terminal registered under `session_key`. Called
    /// from the ACP runtime's tail cleanup so per-session child
    /// processes never outlive the agent connection.
    pub async fn drain_for(&self, session_key: impl AsRef<str>) {
        let sk = session_key.as_ref();
        let drained: Vec<_> = {
            let mut registry = self.registry.lock().await;
            let keys: Vec<_> = registry.keys().filter(|(s, _)| s == sk).cloned().collect();
            keys.into_iter().filter_map(|k| registry.remove(&k)).collect()
        };
        for mut state in drained {
            if let Some(mut child) = state.child.take() {
                let _ = child.start_kill();
                let _ = child.wait().await;
            }
        }
    }
}

struct BufferReaderConfig {
    buffer: Arc<Mutex<OutputBuffer>>,
    limit: u64,
    events_tx: broadcast::Sender<TerminalToolEvent>,
    session_key: String,
    terminal_id: String,
    stream: TerminalToolStream,
}

fn spawn_buffer_reader<R>(reader: R, cfg: BufferReaderConfig)
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    let BufferReaderConfig {
        buffer,
        limit,
        events_tx,
        session_key,
        terminal_id,
        stream,
    } = cfg;
    tokio::spawn(async move {
        let mut reader = reader;
        let mut chunk = [0u8; READ_CHUNK];
        let mut total: u64 = 0;
        loop {
            match reader.read(&mut chunk).await {
                Ok(0) => break,
                Ok(n) => {
                    total = total.saturating_add(n as u64);
                    tracing::trace!(chunk_len = n, total_bytes = total, "tools::terminal: pipe chunk");
                    buffer.lock().await.push(&chunk[..n], limit);
                    let data = String::from_utf8_lossy(&chunk[..n]).into_owned();
                    let _ = events_tx.send(TerminalToolEvent {
                        session_key: session_key.clone(),
                        terminal_id: terminal_id.clone(),
                        kind: TerminalToolEventKind::Output { stream, data },
                    });
                }
                Err(err) => {
                    tracing::debug!(%err, total_bytes = total, "tools::terminal: pipe read failed");
                    break;
                }
            }
        }
    });
}

#[cfg(unix)]
fn exit_status_from(out: &std::process::ExitStatus) -> TerminalExitStatus {
    use std::os::unix::process::ExitStatusExt;
    let mut status = TerminalExitStatus::new();
    if let Some(code) = out.code() {
        status = status.exit_code(code as u32);
    }
    if let Some(sig) = out.signal() {
        status = status.signal(format!("{sig}"));
    }
    status
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::schema::SessionId;

    fn mk(dir: &std::path::Path) -> Terminals {
        Terminals::new(Sandbox::new(dir).expect("sandbox"))
    }

    fn session_key(id: &SessionId) -> &str {
        id.0.as_ref()
    }

    #[tokio::test]
    async fn create_captures_stdout_within_limit() {
        let dir = tempfile::tempdir().unwrap();
        let terms = mk(dir.path());
        let sid = SessionId::new("s");

        let mut req = CreateTerminalRequest::new(sid.clone(), "sh");
        req.args = vec!["-c".into(), "printf 'hello-world'".into()];
        req.output_byte_limit = Some(5);

        let resp = terms.create(session_key(&sid), req).await.expect("spawn ok");

        let wait = WaitForTerminalExitRequest::new(sid.clone(), resp.terminal_id.clone());
        let _ = terms.wait(session_key(&sid), wait).await.expect("exit ok");

        let out_req = TerminalOutputRequest::new(sid.clone(), resp.terminal_id.clone());
        let out = terms.output(session_key(&sid), out_req).await.expect("output ok");
        assert!(out.truncated, "buffer should truncate at limit=5");
        assert!(out.output.len() <= 5);
    }

    #[tokio::test]
    async fn release_drops_state() {
        let dir = tempfile::tempdir().unwrap();
        let terms = mk(dir.path());
        let sid = SessionId::new("s");

        let mut req = CreateTerminalRequest::new(sid.clone(), "sh");
        req.args = vec!["-c".into(), "sleep 5".into()];
        let resp = terms.create(session_key(&sid), req).await.expect("spawn ok");

        let rel = ReleaseTerminalRequest::new(sid.clone(), resp.terminal_id.clone());
        terms.release(session_key(&sid), rel).await.expect("release ok");
        assert!(terms.registry.lock().await.is_empty());
    }

    #[tokio::test]
    async fn drain_for_session_clears_registry() {
        let dir = tempfile::tempdir().unwrap();
        let terms = mk(dir.path());
        let sid = SessionId::new("doomed");

        let mut req = CreateTerminalRequest::new(sid.clone(), "sh");
        req.args = vec!["-c".into(), "sleep 30".into()];
        let resp = terms.create(session_key(&sid), req).await.expect("spawn ok");

        assert!(terms
            .registry
            .lock()
            .await
            .contains_key(&(sid.0.to_string(), resp.terminal_id.clone())));

        terms.drain_for(session_key(&sid)).await;
        assert!(terms.registry.lock().await.is_empty());
    }

    #[tokio::test]
    async fn output_unknown_terminal() {
        let dir = tempfile::tempdir().unwrap();
        let terms = mk(dir.path());
        let sid = SessionId::new("s");
        let req = TerminalOutputRequest::new(sid.clone(), TerminalId::new("nope"));
        let err = terms.output(session_key(&sid), req).await.expect_err("must fail");
        assert!(matches!(err, TerminalError::UnknownTerminal(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn subscribers_receive_output_and_exit() {
        let dir = tempfile::tempdir().unwrap();
        let terms = mk(dir.path());
        let sid = SessionId::new("s");
        let mut rx = terms.subscribe();

        let mut req = CreateTerminalRequest::new(sid.clone(), "sh");
        req.args = vec!["-c".into(), "printf 'hi'".into()];
        let resp = terms.create(session_key(&sid), req).await.expect("spawn ok");
        let term_id = resp.terminal_id.0.to_string();

        // Drain output / exit events with a generous timeout so we
        // tolerate child-startup latency on slow runners.
        let collected = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            let mut events: Vec<TerminalToolEvent> = Vec::new();
            // Drive the wait concurrently so the Exit event can land.
            let wait = WaitForTerminalExitRequest::new(sid.clone(), resp.terminal_id.clone());
            let _exit = terms.wait(session_key(&sid), wait).await.expect("exit ok");
            // After exit we should already have at least one Output + one Exit.
            while let Ok(evt) = tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv()).await {
                if let Ok(evt) = evt {
                    events.push(evt);
                }
            }
            events
        })
        .await
        .expect("events drained");

        assert!(
            collected.iter().all(|e| e.terminal_id == term_id),
            "all events stamped with the spawned terminal id"
        );
        assert!(
            collected
                .iter()
                .any(|e| matches!(e.kind, TerminalToolEventKind::Output { .. })),
            "saw at least one Output event"
        );
        assert!(
            collected
                .iter()
                .any(|e| matches!(e.kind, TerminalToolEventKind::Exit { .. })),
            "saw an Exit event"
        );
    }
}
