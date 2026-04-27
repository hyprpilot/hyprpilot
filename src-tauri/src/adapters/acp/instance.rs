//! `AcpInstance` — owner of one ACP-speaking agent subprocess: the
//! handle the registry keeps + the actor body that drives `initialize`
//! → `session/new` → `session/prompt` and forwards `SessionUpdate`s
//! to the registry's broadcast.
//!
//! `AcpInstance::start(...)` is the constructor (symmetric with
//! `AcpInstance::shutdown` from `InstanceActor`). It spawns the
//! long-lived task and returns the handle. The body, the
//! subprocess-spawn helper, and the prompt-block encoder all live
//! private to this module — they were once in separate
//! `runtime.rs` / `spawn.rs` files; consolidated so the actor's
//! lifecycle reads top-to-bottom in one place.

use std::sync::Arc;
use std::time::Duration;

use agent_client_protocol::schema::{
    CancelNotification, ClientCapabilities, ContentBlock, EmbeddedResource, EmbeddedResourceResource,
    FileSystemCapabilities, InitializeRequest, ListSessionsRequest, ListSessionsResponse, LoadSessionRequest,
    NewSessionRequest, PromptRequest, ProtocolVersion, SessionId, TextContent, TextResourceContents,
};
use agent_client_protocol::{ByteStreams, Client};
use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use tracing::{debug, error, info, warn};

use super::agents::{match_provider_agent, SystemPromptInjection};
use super::client::{AcpClient, ClientEvent, SessionUpdateNotification};
use crate::adapters::instance::{InstanceActor, InstanceInfo, InstanceKey};
use crate::adapters::permission::PermissionController;
use crate::adapters::profile::ResolvedInstance;
use crate::adapters::transcript::Attachment;
use crate::adapters::{Bootstrap, InstanceEvent, InstanceState, TerminalChunk};
use crate::config::{AgentConfig, ProfileConfig};
use crate::tools::{TerminalToolEventKind, TerminalToolStream};

/// How long the registry waits for the actor to ack a `Shutdown`
/// command before dropping the handle.
const SHUTDOWN_ACK_TIMEOUT: Duration = Duration::from_secs(2);

/// Register a typed `on_receive_request` handler that delegates to an
/// async `AcpClient` method returning `Result<Response,
/// agent_client_protocol::Error>`. One registration line per method
/// keeps the handler chain legible.
macro_rules! register_client_handler {
    ($builder:expr, $client:expr, $method:ident) => {{
        let client = $client.clone();
        $builder.on_receive_request(
            move |req, responder: agent_client_protocol::Responder<_>, _cx| {
                let client = client.clone();
                async move { responder.respond_with_result(client.$method(&req).await) }
            },
            agent_client_protocol::on_receive_request!(),
        )
    }};
}

/// Commands the per-instance actor accepts. The actor keeps state
/// internal; this enum is the only public surface the dispatcher
/// uses to drive it.
#[derive(Debug)]
pub enum InstanceCommand {
    Prompt {
        text: String,
        attachments: Vec<Attachment>,
        reply: oneshot::Sender<Result<(), String>>,
    },
    Cancel {
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Ask the agent for its persisted session index. Works in any
    /// bootstrap mode — the actor is always past `initialize` by the
    /// time it processes commands.
    ListSessions {
        cwd: Option<std::path::PathBuf>,
        reply: oneshot::Sender<Result<ListSessionsResponse, String>>,
    },
    /// Shutdown hook — stops the actor after the current prompt
    /// (or immediately if idle). Reply carries the final state.
    Shutdown {
        reply: oneshot::Sender<()>,
    },
}

/// Project an ACP `session/update` notification (as raw JSON, since
/// `TolerantSessionNotification` carries the payload untyped) into a
/// typed `TranscriptItem`. Returns the item the daemon publishes via
/// `InstanceEvent::Transcript`.
///
/// Unknown / future variants land as `TranscriptItem::Unknown`
/// carrying the raw `sessionUpdate` discriminator + payload — the UI
/// dispatches on `item.kind` (`unknown`) and can render a placeholder
/// or sub-dispatch on `item.kind` from the payload. Forward-compat
/// without bricking sessions.
pub(crate) fn map_session_update(update: serde_json::Value) -> crate::adapters::TranscriptItem {
    use crate::adapters::{
        PermissionRequestRecord, PlanRecord, PlanStep, ToolCallContentItem, ToolCallRecord, ToolCallState,
        ToolCallUpdateRecord, TranscriptItem,
    };

    let kind = update
        .get("sessionUpdate")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    fn chunk_text(update: &serde_json::Value) -> String {
        update
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    }

    fn parse_tool_state(s: Option<&str>) -> Option<ToolCallState> {
        match s? {
            "pending" => Some(ToolCallState::Pending),
            "in_progress" | "running" => Some(ToolCallState::Running),
            "completed" => Some(ToolCallState::Completed),
            "failed" => Some(ToolCallState::Failed),
            _ => None,
        }
    }

    fn parse_content(raw: &serde_json::Value) -> Vec<ToolCallContentItem> {
        raw.as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|piece| {
                        let kind = piece.get("type").and_then(|v| v.as_str())?;
                        match kind {
                            "content" | "text" => Some(ToolCallContentItem::Text {
                                text: piece
                                    .get("text")
                                    .and_then(|v| v.as_str())
                                    .or_else(|| {
                                        piece
                                            .get("content")
                                            .and_then(|c| c.get("text"))
                                            .and_then(|v| v.as_str())
                                    })
                                    .unwrap_or("")
                                    .to_string(),
                            }),
                            "diff" | "file" => {
                                let path = piece.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                let snippet = piece
                                    .get("newText")
                                    .or_else(|| piece.get("snippet"))
                                    .and_then(|v| v.as_str())
                                    .map(str::to_string);
                                Some(ToolCallContentItem::File { path, snippet })
                            }
                            _ => Some(ToolCallContentItem::Json { value: piece.clone() }),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn raw_args_summary(raw_input: Option<&serde_json::Value>) -> Option<String> {
        let raw = raw_input?;
        if let Some(cmd) = raw.get("command").and_then(|v| v.as_str()) {
            return Some(cmd.to_string());
        }
        if let Some(path) = raw.get("path").and_then(|v| v.as_str()) {
            return Some(path.to_string());
        }
        serde_json::to_string(raw).ok()
    }

    match kind.as_str() {
        "user_message_chunk" => TranscriptItem::UserText {
            text: chunk_text(&update),
        },
        "agent_message_chunk" => TranscriptItem::AgentText {
            text: chunk_text(&update),
        },
        "agent_thought_chunk" => TranscriptItem::AgentThought {
            text: chunk_text(&update),
        },
        "tool_call" => {
            let id = update
                .get("toolCallId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let tool_kind = update
                .get("kind")
                .and_then(|v| v.as_str())
                .map(str::to_ascii_lowercase)
                .unwrap_or_else(|| "acp".to_string());
            let title = update.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let state =
                parse_tool_state(update.get("status").and_then(|v| v.as_str())).unwrap_or(ToolCallState::Pending);
            let raw_args = raw_args_summary(update.get("rawInput"));
            let content = update.get("content").map(parse_content).unwrap_or_default();
            TranscriptItem::ToolCall(ToolCallRecord {
                id,
                tool_kind,
                title,
                state,
                raw_args,
                content,
            })
        }
        "tool_call_update" => {
            let id = update
                .get("toolCallId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let tool_kind = update.get("kind").and_then(|v| v.as_str()).map(str::to_ascii_lowercase);
            let title = update.get("title").and_then(|v| v.as_str()).map(str::to_string);
            let state = parse_tool_state(update.get("status").and_then(|v| v.as_str()));
            let raw_args = raw_args_summary(update.get("rawInput"));
            let content = update.get("content").map(parse_content).unwrap_or_default();
            TranscriptItem::ToolCallUpdate(ToolCallUpdateRecord {
                id,
                tool_kind,
                title,
                state,
                raw_args,
                content,
            })
        }
        "plan" => {
            let steps = update
                .get("entries")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .map(|entry| PlanStep {
                            content: entry.get("content").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                            priority: entry.get("priority").and_then(|v| v.as_str()).map(str::to_string),
                            status: entry.get("status").and_then(|v| v.as_str()).map(str::to_string),
                        })
                        .collect()
                })
                .unwrap_or_default();
            TranscriptItem::Plan(PlanRecord { steps })
        }
        "permission_request" => {
            let request_id = update
                .get("requestId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let tool = update.get("tool").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let tool_kind = update.get("kind").and_then(|v| v.as_str()).unwrap_or("acp").to_string();
            let args = update.get("args").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let options = update
                .get("options")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();
            TranscriptItem::PermissionRequest(PermissionRequestRecord {
                request_id,
                tool,
                tool_kind,
                args,
                options,
            })
        }
        _ => TranscriptItem::Unknown {
            wire_kind: kind,
            payload: update,
        },
    }
}

/// Build the ACP `Vec<ContentBlock>` payload for one user turn.
/// Attachments project as `EmbeddedResource` and prepend the prose
/// text — order matters per the convention documented on
/// `UserTurnInput`: agents read context before instructions.
pub(crate) fn build_prompt_blocks(text: &str, attachments: &[Attachment]) -> Vec<ContentBlock> {
    let mut blocks: Vec<ContentBlock> = Vec::with_capacity(attachments.len() + 1);
    for att in attachments {
        let mut tr = TextResourceContents::new(att.body.clone(), att.file_uri());
        tr.mime_type = Some(att.mime_type());
        blocks.push(ContentBlock::Resource(EmbeddedResource::new(
            EmbeddedResourceResource::TextResourceContents(tr),
        )));
    }
    blocks.push(ContentBlock::Text(TextContent::new(text.to_owned())));
    blocks
}

/// Captured stdio pair for a freshly-spawned agent subprocess.
struct ChildStdio {
    stdin: ChildStdin,
    stdout: ChildStdout,
}

/// Output of `spawn_subprocess`: the child + its stdio + optional
/// first-message-prefix the runtime prepends to the first
/// `session/prompt` for vendors without a launch-time hook.
struct SpawnedAgent {
    child: Child,
    stdio: ChildStdio,
    stderr: ChildStderr,
    first_message_prefix: Option<String>,
}

/// Spawn the configured agent subprocess. `system_prompt`, when set,
/// is routed through the vendor's `inject_system_prompt` hook —
/// either mutating `cmd` pre-spawn or returning text the runtime
/// prepends onto the first `session/prompt`.
fn spawn_subprocess(cfg: &AgentConfig, system_prompt: Option<&str>) -> Result<SpawnedAgent> {
    info!(
        agent = %cfg.id,
        provider = ?cfg.provider,
        cwd = ?cfg.cwd,
        command = ?cfg.command,
        has_system_prompt = system_prompt.is_some(),
        "acp::instance: launching agent subprocess"
    );

    let agent = match_provider_agent(cfg.provider);
    let mut cmd = agent.spawn(cfg);
    // Centralize stderr capture here rather than duplicating across
    // every vendor agent. Vendor SDKs (notably claude-agent-sdk under
    // claude-code-acp) print noisy cleanup stack traces to stderr on
    // shutdown; piping keeps that out of the parent terminal.
    cmd.stderr(std::process::Stdio::piped());
    let first_message_prefix = match system_prompt {
        Some(prompt) => match agent.inject_system_prompt(&mut cmd, prompt) {
            SystemPromptInjection::Handled => None,
            SystemPromptInjection::FirstMessage(text) => Some(text),
        },
        None => None,
    };
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(err) => {
            error!(agent = %cfg.id, provider = ?cfg.provider, %err, "acp::instance: failed to spawn agent");
            return Err(err)
                .with_context(|| format!("failed to spawn agent '{}' (provider {:?})", cfg.id, cfg.provider));
        }
    };

    let pid = child.id();

    let stdin = match child.stdin.take() {
        Some(s) => s,
        None => bail!("agent '{}' stdin not captured — check Stdio::piped()", cfg.id),
    };
    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => bail!("agent '{}' stdout not captured — check Stdio::piped()", cfg.id),
    };
    let stderr = match child.stderr.take() {
        Some(s) => s,
        None => bail!("agent '{}' stderr not captured — check Stdio::piped()", cfg.id),
    };

    info!(
        agent = %cfg.id,
        pid = ?pid,
        first_message_injection = first_message_prefix.is_some(),
        "acp::instance: agent subprocess spawned"
    );

    Ok(SpawnedAgent {
        child,
        stdio: ChildStdio { stdin, stdout },
        stderr,
        first_message_prefix,
    })
}

/// Handle the registry keeps after `AcpInstance::start`. Dropping it
/// cancels the actor (via the `cmd_tx` drop + the actor's select
/// loop observing `None` from the mpsc receiver).
#[derive(Debug)]
pub struct AcpInstance {
    pub key: InstanceKey,
    pub agent_id: String,
    /// `Some` when a `[[profiles]]` entry resolved during ensure,
    /// `None` for bare-agent resolutions (no profile selected).
    pub profile_id: Option<String>,
    /// Per-instance operational mode. Mirrored onto `InstanceInfo`.
    pub mode: Option<String>,
    pub cmd_tx: mpsc::UnboundedSender<InstanceCommand>,
    /// Populated after the first prompt's `session/new` resolves.
    /// `None` while the instance is still bootstrapping.
    pub session_id: Arc<tokio::sync::RwLock<Option<SessionId>>>,
}

impl AcpInstance {
    pub async fn current_session_id(&self) -> Option<String> {
        self.session_id.read().await.as_ref().map(|id| id.0.to_string())
    }

    /// Spawn the per-instance actor task and return its handle.
    /// Symmetric with [`Self::shutdown`]: the registry calls `start`
    /// to bring an instance up and `shutdown` to tear it down.
    ///
    /// `bootstrap` picks between `session/new` (`Fresh`),
    /// `session/load` (`Resume`), or neither (`ListOnly`). The actor
    /// publishes lifecycle + transcript + permission events onto
    /// `events_tx`.
    ///
    /// `mcps_override` is the per-instance MCP enabled-list override;
    /// `None` falls back to `profile.mcps`. `Some(vec![])` is the
    /// explicit "no MCPs" override.
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn start(
        resolved: ResolvedInstance,
        key: InstanceKey,
        profile_id: Option<String>,
        events_tx: broadcast::Sender<InstanceEvent>,
        bootstrap: Bootstrap,
        permissions: Arc<dyn PermissionController>,
        profile: Option<ProfileConfig>,
        mcps_override: Option<Vec<String>>,
    ) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<InstanceCommand>();
        let initial = match &bootstrap {
            Bootstrap::Resume(id) => Some(SessionId::new(id.clone())),
            Bootstrap::Fresh | Bootstrap::ListOnly => None,
        };
        let session_id = Arc::new(tokio::sync::RwLock::new(initial));
        let mode = resolved.mode.clone();
        let instance_id = key.as_string();

        // Mode is a per-instance operational override (e.g.
        // claude-code's `plan` / `edit`). Surface it so UI pickers
        // see it; vendor-specific wire injection lands in the agent
        // impl (today only logged here).
        if let Some(m) = &mode {
            tracing::info!(
                agent = %resolved.agent.id,
                instance = %instance_id,
                mode = %m,
                "acp::instance: mode set"
            );
        }

        // Effective MCP list: per-instance override wins, else profile.mcps.
        // Vendor-specific wire injection at spawn is incremental per
        // vendor — log the intent for now (K-270).
        let effective_mcps = mcps_override.or_else(|| profile.as_ref().and_then(|p| p.mcps.clone()));
        if let Some(names) = &effective_mcps {
            tracing::warn!(
                agent = %resolved.agent.id,
                instance = %instance_id,
                mcps = ?names,
                "acp::instance: MCP enabled-list resolved but vendor injection not yet wired — see K-270"
            );
        }

        let instance = AcpInstance {
            key,
            agent_id: resolved.agent.id.clone(),
            profile_id,
            mode,
            cmd_tx,
            session_id: session_id.clone(),
        };

        tokio::spawn(run(
            resolved,
            instance_id,
            cmd_rx,
            events_tx,
            session_id,
            bootstrap,
            permissions,
            profile,
        ));

        instance
    }
}

#[async_trait]
impl InstanceActor for AcpInstance {
    fn info(&self) -> InstanceInfo {
        // session_id read is sync-safe on the RwLock's try_read, but
        // we don't need it here — the registry's list path populates
        // it async via `current_session_id` when it matters. Keep
        // this call sync so the generic registry can assemble the
        // snapshot without an async fn on the trait.
        let session_id = self
            .session_id
            .try_read()
            .ok()
            .and_then(|s| s.as_ref().map(|id| id.0.to_string()));
        InstanceInfo {
            id: self.key.as_string(),
            agent_id: self.agent_id.clone(),
            profile_id: self.profile_id.clone(),
            session_id,
            mode: self.mode.clone(),
        }
    }

    async fn shutdown(&self) {
        let (tx, rx) = oneshot::channel();
        if self.cmd_tx.send(InstanceCommand::Shutdown { reply: tx }).is_err() {
            return;
        }
        let _ = tokio::time::timeout(SHUTDOWN_ACK_TIMEOUT, rx).await;
    }
}

/// The long-lived actor body. Owns the ACP `ConnectionTo<Agent>`,
/// the child process, the dispatch loop. Spawned by
/// [`AcpInstance::start`].
#[allow(clippy::too_many_arguments)]
async fn run(
    resolved: ResolvedInstance,
    instance_id: String,
    mut cmd_rx: mpsc::UnboundedReceiver<InstanceCommand>,
    events_tx: broadcast::Sender<InstanceEvent>,
    session_id_slot: Arc<tokio::sync::RwLock<Option<SessionId>>>,
    bootstrap: Bootstrap,
    permissions: Arc<dyn PermissionController>,
    profile: Option<ProfileConfig>,
) {
    let agent_id = resolved.agent.id.clone();
    let _ = events_tx.send(InstanceEvent::State {
        agent_id: agent_id.clone(),
        instance_id: instance_id.clone(),
        session_id: None,
        state: InstanceState::Starting,
    });

    let cfg = {
        let mut cfg = resolved.agent.clone();
        cfg.model = resolved.model.clone();
        cfg
    };
    let system_prompt = resolved.system_prompt.clone();

    let (mut child, stdio, stderr, mut first_message_prefix) = match spawn_subprocess(&cfg, system_prompt.as_deref()) {
        Ok(spawned) => (
            spawned.child,
            spawned.stdio,
            spawned.stderr,
            spawned.first_message_prefix,
        ),
        Err(err) => {
            error!(agent = %agent_id, %err, "acp::instance: spawn failed");
            let _ = events_tx.send(InstanceEvent::State {
                agent_id,
                instance_id,
                session_id: None,
                state: InstanceState::Error,
            });
            return;
        }
    };

    // Drain the subprocess's stderr into tracing so vendor-SDK cleanup
    // noise lands in our rolling log file instead of the parent
    // terminal. Each line goes through at `info!` with an `agent_stderr`
    // target so users can filter via
    // `RUST_LOG=hyprpilot=info,agent_stderr=warn`. Task ends on stream
    // close (child exit).
    {
        use tokio::io::{AsyncBufReadExt, BufReader};
        let agent_for_stderr = agent_id.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        tracing::info!(target: "agent_stderr", agent = %agent_for_stderr, "{line}");
                    }
                    Ok(None) => break,
                    Err(err) => {
                        tracing::warn!(
                            target: "agent_stderr",
                            agent = %agent_for_stderr,
                            %err,
                            "stderr read error"
                        );
                        break;
                    }
                }
            }
        });
    }

    // Tee stdout → tracing + ACP transport. Stdout IS the ACP wire
    // channel so we can't just redirect it; we read each line, emit
    // it at `trace!` target `agent_stdout`, then forward the original
    // bytes into a duplex pipe the transport reads from. Filter in
    // with `RUST_LOG=agent_stdout=trace`; noisy (every JSON-RPC
    // frame) so `trace` is deliberately opt-in.
    let transport_stdout = {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        let (mut tee_writer, tee_reader) = tokio::io::duplex(64 * 1024);
        let agent_for_stdout = agent_id.clone();
        let child_stdout = stdio.stdout;
        tokio::spawn(async move {
            let mut reader = BufReader::new(child_stdout);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => break,
                    Ok(_) => {
                        let trimmed = line.trim_end_matches(['\n', '\r']);
                        if !trimmed.is_empty() {
                            tracing::trace!(target: "agent_stdout", agent = %agent_for_stdout, "{trimmed}");
                        }
                        if let Err(err) = tee_writer.write_all(line.as_bytes()).await {
                            tracing::warn!(
                                target: "agent_stdout",
                                agent = %agent_for_stdout,
                                %err,
                                "tee forward failed"
                            );
                            break;
                        }
                    }
                    Err(err) => {
                        tracing::warn!(
                            target: "agent_stdout",
                            agent = %agent_for_stdout,
                            %err,
                            "stdout read error"
                        );
                        break;
                    }
                }
            }
        });
        tee_reader
    };

    let (client_events_tx, mut client_events_rx) = mpsc::unbounded_channel::<ClientEvent>();
    let sandbox_root = cfg
        .cwd
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| "/".into()));
    let client = match AcpClient::with_instance_id(
        client_events_tx,
        sandbox_root,
        permissions.clone(),
        profile.clone(),
        Some(instance_id.clone()),
    ) {
        Ok(c) => c,
        Err(err) => {
            error!(agent = %agent_id, %err, "acp::instance: sandbox init failed");
            let _ = events_tx.send(InstanceEvent::State {
                agent_id,
                instance_id,
                session_id: None,
                state: InstanceState::Error,
            });
            return;
        }
    };

    let transport = ByteStreams::new(stdio.stdin.compat_write(), transport_stdout.compat());

    let events_tx_notif = events_tx.clone();
    let agent_id_notif = agent_id.clone();
    let instance_id_notif = instance_id.clone();
    let session_id_forward = session_id_slot.clone();
    // Tracks the in-flight turn id so the notification / permission
    // arms of the dispatch loop can stamp events with it without
    // re-coordinating with the Prompt-handling task. Set when a
    // `Prompt` is accepted, cleared when the spawned `session/prompt`
    // task replies. `tokio::sync::RwLock` because the spawned task
    // crosses an `.await`; reads from the loop are non-blocking.
    let current_turn_id: Arc<tokio::sync::RwLock<Option<String>>> = Arc::new(tokio::sync::RwLock::new(None));

    // Bridge live terminal output → InstanceEvent::Terminal. The ACP
    // `terminal/output` request remains a polled snapshot path
    // (agent-side); the UI consumes this push stream so it never
    // re-polls.
    {
        let mut rx = client.subscribe_terminals();
        let events_tx = events_tx.clone();
        let agent_id = agent_id.clone();
        let instance_id = instance_id.clone();
        let session_id_slot = session_id_slot.clone();
        let current_turn_id = current_turn_id.clone();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(evt) => {
                        let session_id = match session_id_slot.read().await.clone() {
                            Some(sid) => sid.0.to_string(),
                            None => evt.session_key.clone(),
                        };
                        let turn_id = current_turn_id.read().await.clone();
                        let chunk = match evt.kind {
                            TerminalToolEventKind::Output { stream, data } => TerminalChunk::Output {
                                stream: match stream {
                                    TerminalToolStream::Stdout => crate::adapters::TerminalStream::Stdout,
                                    TerminalToolStream::Stderr => crate::adapters::TerminalStream::Stderr,
                                },
                                data,
                            },
                            TerminalToolEventKind::Exit { exit_code, signal } => {
                                TerminalChunk::Exit { exit_code, signal }
                            }
                        };
                        let _ = events_tx.send(InstanceEvent::Terminal {
                            agent_id: agent_id.clone(),
                            instance_id: instance_id.clone(),
                            session_id,
                            turn_id,
                            terminal_id: evt.terminal_id,
                            chunk,
                        });
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(n, instance = %instance_id, "acp::instance: terminal-event bridge lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
        });
    }

    let dispatch = async move |connection: agent_client_protocol::ConnectionTo<agent_client_protocol::Agent>| {
        debug!(agent = %agent_id_notif, "acp::instance: sending initialize request");
        let init = connection
            .send_request(
                InitializeRequest::new(ProtocolVersion::V1).client_capabilities(
                    ClientCapabilities::new()
                        .fs(FileSystemCapabilities::new().read_text_file(true).write_text_file(true))
                        .terminal(true),
                ),
            )
            .block_task()
            .await?;
        info!(
            agent = %agent_id_notif,
            protocol = ?init.protocol_version,
            load_session = init.agent_capabilities.load_session,
            "acp::instance: initialized"
        );

        let cwd = cfg
            .cwd
            .clone()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| "/".into()));
        let load_supported = init.agent_capabilities.load_session;

        let session_id: Option<SessionId> = match bootstrap {
            Bootstrap::Fresh => {
                debug!(agent = %agent_id_notif, "acp::instance: sending session/new");
                let new_session = connection
                    .send_request(NewSessionRequest::new(cwd.clone()))
                    .block_task()
                    .await?;
                let sid = new_session.session_id.clone();
                info!(
                    agent = %agent_id_notif,
                    instance = %instance_id_notif,
                    session = %sid,
                    "acp::instance: session/new accepted"
                );
                {
                    let mut slot = session_id_forward.write().await;
                    *slot = Some(sid.clone());
                }
                let _ = events_tx_notif.send(InstanceEvent::State {
                    agent_id: agent_id_notif.clone(),
                    instance_id: instance_id_notif.clone(),
                    session_id: Some(sid.0.to_string()),
                    state: InstanceState::Running,
                });
                Some(sid)
            }
            Bootstrap::Resume(sid) => {
                let sid = SessionId::new(sid);
                if !load_supported {
                    warn!(agent = %agent_id_notif, "acp::instance: load_session not advertised by agent");
                    let _ = events_tx_notif.send(InstanceEvent::State {
                        agent_id: agent_id_notif.clone(),
                        instance_id: instance_id_notif.clone(),
                        session_id: Some(sid.0.to_string()),
                        state: InstanceState::Error,
                    });
                    return Err(
                        agent_client_protocol::Error::method_not_found().data(serde_json::json!({
                            "reason": format!("{}: load_session not supported", agent_id_notif),
                        })),
                    );
                }
                {
                    let mut slot = session_id_forward.write().await;
                    *slot = Some(sid.clone());
                }
                debug!(agent = %agent_id_notif, session = %sid, "acp::instance: sending session/load");
                if let Err(err) = connection
                    .send_request(LoadSessionRequest::new(sid.clone(), cwd.clone()))
                    .block_task()
                    .await
                {
                    warn!(agent = %agent_id_notif, %err, "acp::instance: load_session failed");
                    let _ = events_tx_notif.send(InstanceEvent::State {
                        agent_id: agent_id_notif.clone(),
                        instance_id: instance_id_notif.clone(),
                        session_id: Some(sid.0.to_string()),
                        state: InstanceState::Error,
                    });
                    return Err(err);
                }
                info!(
                    agent = %agent_id_notif,
                    instance = %instance_id_notif,
                    session = %sid,
                    "acp::instance: session/load accepted"
                );
                let _ = events_tx_notif.send(InstanceEvent::State {
                    agent_id: agent_id_notif.clone(),
                    instance_id: instance_id_notif.clone(),
                    session_id: Some(sid.0.to_string()),
                    state: InstanceState::Running,
                });
                Some(sid)
            }
            Bootstrap::ListOnly => {
                let _ = events_tx_notif.send(InstanceEvent::State {
                    agent_id: agent_id_notif.clone(),
                    instance_id: instance_id_notif.clone(),
                    session_id: None,
                    state: InstanceState::Running,
                });
                None
            }
        };

        loop {
            tokio::select! {
                cmd = cmd_rx.recv() => {
                    let Some(cmd) = cmd else {
                        info!(agent = %agent_id_notif, "acp::instance: command channel closed, shutting down");
                        break;
                    };
                    match cmd {
                        // Detached: awaiting `send_request(...).block_task()` inline here
                        // blocks the select! from pumping `client_events_rx`, so every
                        // `SessionNotification` (and every `PermissionRequest`!) queues
                        // on the mpsc until the prompt resolves. The permission path
                        // blocks for up to 10min waiting on a UI reply — but the UI
                        // never sees the prompt because the event is stuck in that same
                        // mpsc. Spawn the request so the loop keeps draining.
                        InstanceCommand::Prompt { text, attachments, reply } => {
                            let Some(sid) = session_id.clone() else {
                                let _ = reply.send(Err("no live session in list-only actor".into()));
                                continue;
                            };
                            let text = match first_message_prefix.take() {
                                Some(prefix) => format!("{prefix}\n\n{text}"),
                                None => text,
                            };
                            let turn_id = uuid::Uuid::new_v4().to_string();
                            info!(
                                agent = %agent_id_notif,
                                session = %sid,
                                turn = %turn_id,
                                text_len = text.len(),
                                attachments = attachments.len(),
                                "acp::instance: turn start (session/prompt)"
                            );
                            *current_turn_id.write().await = Some(turn_id.clone());
                            let _ = events_tx_notif.send(InstanceEvent::TurnStarted {
                                agent_id: agent_id_notif.clone(),
                                instance_id: instance_id_notif.clone(),
                                session_id: sid.0.to_string(),
                                turn_id: turn_id.clone(),
                            });
                            // Daemon-authoritative user-prompt transcript item:
                            // emitted at submit time so the UI no longer mirrors
                            // optimistically. Single source of truth for the user
                            // turn's text + attachments.
                            let _ = events_tx_notif.send(InstanceEvent::Transcript {
                                agent_id: agent_id_notif.clone(),
                                instance_id: instance_id_notif.clone(),
                                session_id: sid.0.to_string(),
                                turn_id: Some(turn_id.clone()),
                                item: crate::adapters::TranscriptItem::UserPrompt {
                                    text: text.clone(),
                                    attachments: attachments.clone(),
                                },
                            });
                            let blocks = build_prompt_blocks(&text, &attachments);
                            let conn = connection.clone();
                            let agent_log = agent_id_notif.clone();
                            let session_log = sid.clone();
                            let events_tx_done = events_tx_notif.clone();
                            let current_turn_done = current_turn_id.clone();
                            let agent_id_done = agent_id_notif.clone();
                            let instance_id_done = instance_id_notif.clone();
                            let turn_id_done = turn_id.clone();
                            tokio::spawn(async move {
                                let res = conn
                                    .send_request(PromptRequest::new(sid.clone(), blocks))
                                    .block_task()
                                    .await;
                                let (stop_reason, mapped) = match res {
                                    Ok(resp) => {
                                        info!(
                                            agent = %agent_log,
                                            session = %session_log,
                                            turn = %turn_id_done,
                                            stop_reason = ?resp.stop_reason,
                                            "acp::instance: turn stop (prompt resolved)"
                                        );
                                        let stop = serde_json::to_value(resp.stop_reason)
                                            .ok()
                                            .and_then(|v| v.as_str().map(str::to_owned));
                                        (stop, Ok(()))
                                    }
                                    Err(err) => {
                                        warn!(
                                            agent = %agent_log,
                                            session = %session_log,
                                            turn = %turn_id_done,
                                            %err,
                                            "acp::instance: turn ended with error"
                                        );
                                        (None, Err(err.to_string()))
                                    }
                                };
                                {
                                    let mut slot = current_turn_done.write().await;
                                    if slot.as_deref() == Some(turn_id_done.as_str()) {
                                        *slot = None;
                                    }
                                }
                                let _ = events_tx_done.send(InstanceEvent::TurnEnded {
                                    agent_id: agent_id_done,
                                    instance_id: instance_id_done,
                                    session_id: sid.0.to_string(),
                                    turn_id: turn_id_done,
                                    stop_reason,
                                });
                                let _ = reply.send(mapped);
                            });
                        }
                        InstanceCommand::Cancel { reply } => {
                            let Some(sid) = session_id.clone() else {
                                let _ = reply.send(Err("no live session in list-only actor".into()));
                                continue;
                            };
                            info!(
                                agent = %agent_id_notif,
                                session = %sid,
                                "acp::instance: turn cancel (CancelNotification)"
                            );
                            let res = connection
                                .send_notification(CancelNotification::new(sid))
                                .map_err(|e| e.to_string());
                            let _ = reply.send(res);
                        }
                        // Detached for the same reason as Prompt: list_sessions can take
                        // seconds against a remote index, and blocking the select! starves
                        // event pumping.
                        InstanceCommand::ListSessions { cwd: filter_cwd, reply } => {
                            debug!(
                                agent = %agent_id_notif,
                                cwd_filter = ?filter_cwd,
                                "acp::instance: session/list requested"
                            );
                            let conn = connection.clone();
                            tokio::spawn(async move {
                                let mut req = ListSessionsRequest::new();
                                if let Some(c) = filter_cwd {
                                    req = req.cwd(c);
                                }
                                let res = conn
                                    .send_request(req)
                                    .block_task()
                                    .await
                                    .map_err(|e| e.to_string());
                                let _ = reply.send(res);
                            });
                        }
                        InstanceCommand::Shutdown { reply } => {
                            info!(
                                agent = %agent_id_notif,
                                instance = %instance_id_notif,
                                has_session = session_id.is_some(),
                                reason = "shutdown command received",
                                "acp::instance: shutting down instance"
                            );
                            if let Some(sid) = session_id.clone() {
                                let _ = connection.send_notification(CancelNotification::new(sid));
                            }
                            let _ = reply.send(());
                            break;
                        }
                    }
                }
                evt = client_events_rx.recv() => {
                    let Some(evt) = evt else { break };
                    match evt {
                        ClientEvent::Notification(SessionUpdateNotification { session_id: sid, update }) => {
                            let update_kind = update
                                .get("sessionUpdate")
                                .and_then(|v| v.as_str())
                                .unwrap_or("<unknown>")
                                .to_string();
                            if update_kind == "agent_message_chunk" || update_kind == "user_message_chunk" {
                                let chunk_len = update
                                    .get("content")
                                    .and_then(|c| c.get("text"))
                                    .and_then(|v| v.as_str())
                                    .map(str::len)
                                    .unwrap_or(0);
                                tracing::trace!(
                                    agent = %agent_id_notif,
                                    session = %sid,
                                    update_kind,
                                    chunk_len,
                                    "acp::instance: session/update text chunk"
                                );
                            } else {
                                debug!(
                                    agent = %agent_id_notif,
                                    session = %sid,
                                    update_kind,
                                    "acp::instance: session/update received"
                                );
                            }
                            let item = map_session_update(update);
                            let turn_id = current_turn_id.read().await.clone();
                            let _ = events_tx_notif.send(InstanceEvent::Transcript {
                                agent_id: agent_id_notif.clone(),
                                instance_id: instance_id_notif.clone(),
                                session_id: sid,
                                turn_id,
                                item,
                            });
                        }
                        ClientEvent::PermissionRequested {
                            session_id: sid,
                            request_id,
                            tool,
                            kind,
                            args,
                            options,
                        } => {
                            debug!(
                                agent = %agent_id_notif,
                                session = %sid,
                                request_id,
                                tool = %tool,
                                "acp::instance: fan out permission prompt to UI"
                            );
                            let turn_id = current_turn_id.read().await.clone();
                            let _ = events_tx_notif.send(InstanceEvent::PermissionRequest {
                                agent_id: agent_id_notif.clone(),
                                instance_id: instance_id_notif.clone(),
                                session_id: sid,
                                turn_id,
                                request_id,
                                tool,
                                kind,
                                args,
                                options,
                            });
                        }
                    }
                }
            }
        }
        Ok::<(), agent_client_protocol::Error>(())
    };

    let builder = Client.builder().on_receive_notification(
        {
            let client = client.clone();
            move |notification: SessionUpdateNotification, _cx| {
                let client = client.clone();
                async move {
                    client.forward_notification(notification);
                    Ok(())
                }
            }
        },
        agent_client_protocol::on_receive_notification!(),
    );
    let builder = register_client_handler!(builder, client, request_permission);
    let builder = register_client_handler!(builder, client, read_text_file);
    let builder = register_client_handler!(builder, client, write_text_file);
    let builder = register_client_handler!(builder, client, create_terminal);
    let builder = register_client_handler!(builder, client, terminal_output);
    let builder = register_client_handler!(builder, client, wait_for_terminal_exit);
    let builder = register_client_handler!(builder, client, kill_terminal);
    let builder = register_client_handler!(builder, client, release_terminal);

    let run_outcome = builder.connect_with(transport, dispatch).await;

    let final_state = match &run_outcome {
        Ok(_) => {
            info!(agent = %agent_id, "acp::instance: instance ended cleanly");
            InstanceState::Ended
        }
        Err(err) => {
            warn!(agent = %agent_id, %err, "acp::instance: instance ended with error");
            InstanceState::Error
        }
    };

    // Give the agent subprocess a brief window to exit cleanly after
    // the transport closes above. The `CancelNotification` we sent on
    // shutdown + the resulting stdin EOF are the standard ACP signals
    // to terminate. SIGKILL'ing zero-delay mid-cleanup makes vendor
    // SDKs (notably `@anthropic-ai/claude-agent-sdk` inside
    // claude-code-acp) spew "Query closed before response received" on
    // stderr because they're tearing down a still-open Anthropic
    // streaming connection that's kept warm between turns. Wait up to
    // 5s for a clean exit, fall back to SIGKILL.
    match tokio::time::timeout(std::time::Duration::from_secs(5), child.wait()).await {
        Ok(Ok(status)) => debug!(agent = %agent_id, ?status, "acp::instance: child exited cleanly"),
        Ok(Err(err)) => warn!(agent = %agent_id, %err, "acp::instance: child wait failed"),
        Err(_) => {
            warn!(
                agent = %agent_id,
                "acp::instance: child did not exit within 5s after stdin EOF, sending SIGKILL"
            );
            let _ = child.kill().await;
        }
    }
    let sid = session_id_slot.read().await.clone();
    if let Some(ref id) = sid {
        client.drain_terminals_for_session(id).await;
    }
    let _ = events_tx.send(InstanceEvent::State {
        agent_id,
        instance_id,
        session_id: sid.as_ref().map(|id| id.0.to_string()),
        state: final_state,
    });
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::adapters::permission::DefaultPermissionController;
    use crate::config::{AgentConfig, AgentProvider};

    #[test]
    fn build_prompt_blocks_emits_only_text_when_no_attachments() {
        let blocks = build_prompt_blocks("hello", &[]);
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            ContentBlock::Text(t) => assert_eq!(t.text, "hello"),
            other => panic!("expected text block, got {other:?}"),
        }
    }

    #[test]
    fn build_prompt_blocks_prepends_resources_before_text() {
        let att = Attachment {
            slug: "git-commit".into(),
            path: PathBuf::from("/tmp/skills/git-commit/SKILL.md"),
            body: "stage and commit".into(),
            title: Some("Git commit".into()),
        };
        let blocks = build_prompt_blocks("please commit", std::slice::from_ref(&att));
        assert_eq!(blocks.len(), 2, "one resource + one text");
        let ContentBlock::Resource(res) = &blocks[0] else {
            panic!("first block must be Resource");
        };
        let EmbeddedResourceResource::TextResourceContents(tr) = &res.resource else {
            panic!("resource must carry text contents");
        };
        assert_eq!(tr.uri, "file:///tmp/skills/git-commit/SKILL.md");
        assert_eq!(tr.mime_type.as_deref(), Some("text/markdown"));
        assert_eq!(tr.text, "stage and commit");
        match &blocks[1] {
            ContentBlock::Text(t) => assert_eq!(t.text, "please commit"),
            other => panic!("second block must be text, got {other:?}"),
        }
    }

    #[test]
    fn build_prompt_blocks_preserves_attachment_order() {
        let a = Attachment {
            slug: "a".into(),
            path: PathBuf::from("/tmp/a/SKILL.md"),
            body: "A".into(),
            title: None,
        };
        let b = Attachment {
            slug: "b".into(),
            path: PathBuf::from("/tmp/b/SKILL.md"),
            body: "B".into(),
            title: None,
        };
        let blocks = build_prompt_blocks("text", &[a, b]);
        assert_eq!(blocks.len(), 3);
        let ContentBlock::Resource(first) = &blocks[0] else {
            panic!()
        };
        let EmbeddedResourceResource::TextResourceContents(tr0) = &first.resource else {
            panic!()
        };
        assert_eq!(tr0.text, "A");
        let ContentBlock::Resource(second) = &blocks[1] else {
            panic!()
        };
        let EmbeddedResourceResource::TextResourceContents(tr1) = &second.resource else {
            panic!()
        };
        assert_eq!(tr1.text, "B");
    }

    fn dummy_resolved(id: &str) -> ResolvedInstance {
        ResolvedInstance {
            agent: AgentConfig {
                id: id.into(),
                provider: AgentProvider::AcpClaudeCode,
                command: Some("/bin/false".into()),
                args: Vec::new(),
                cwd: None,
                env: Default::default(),
                model: None,
            },
            profile_id: None,
            model: None,
            system_prompt: None,
            mode: None,
        }
    }

    fn dummy_permissions() -> Arc<dyn PermissionController> {
        Arc::new(DefaultPermissionController::new()) as Arc<dyn PermissionController>
    }

    /// Regression: starting against a child that exits immediately
    /// pushes an `Error` lifecycle event rather than hanging forever.
    /// Smoke-tests the actor shell without depending on a real agent.
    #[tokio::test(flavor = "multi_thread")]
    async fn dead_child_yields_error_state() {
        let (tx, mut rx) = broadcast::channel(8);
        let handle = AcpInstance::start(
            dummy_resolved("ded"),
            crate::adapters::InstanceKey::new_v4(),
            None,
            tx,
            Bootstrap::Fresh,
            dummy_permissions(),
            None,
            None,
        );

        let first = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("starting event timely")
            .expect("starting event arrives");
        match first {
            InstanceEvent::State {
                state: InstanceState::Starting,
                ..
            } => {}
            other => panic!("expected Starting, got {other:?}"),
        }

        let err = tokio::time::timeout(std::time::Duration::from_secs(15), async {
            loop {
                match rx.recv().await {
                    Ok(InstanceEvent::State {
                        state: InstanceState::Error,
                        ..
                    }) => return Ok::<(), ()>(()),
                    Ok(InstanceEvent::State {
                        state: InstanceState::Ended,
                        ..
                    }) => return Ok(()),
                    Ok(_) => continue,
                    Err(_) => return Err(()),
                }
            }
        })
        .await
        .expect("actor settles");
        assert!(err.is_ok(), "actor reached terminal state");

        drop(handle);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cancel_against_dead_session_does_not_panic() {
        let (tx, _rx) = broadcast::channel(8);
        let handle = AcpInstance::start(
            dummy_resolved("ded-cancel"),
            crate::adapters::InstanceKey::new_v4(),
            None,
            tx,
            Bootstrap::Fresh,
            dummy_permissions(),
            None,
            None,
        );

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let (reply_tx, reply_rx) = oneshot::channel();
        let _ = handle.cmd_tx.send(InstanceCommand::Cancel { reply: reply_tx });
        let _ = tokio::time::timeout(std::time::Duration::from_secs(1), reply_rx).await;
    }

    /// Smoke: a `ListOnly` actor against a dead child still settles
    /// (the `initialize` roundtrip fails, which drives the actor to
    /// `Error` instead of panicking or hanging). The real list-only
    /// path is exercised end-to-end against the mock ACP agent.
    #[tokio::test(flavor = "multi_thread")]
    async fn list_only_against_dead_child_settles() {
        let (tx, mut rx) = broadcast::channel(8);
        let handle = AcpInstance::start(
            dummy_resolved("ded-list"),
            crate::adapters::InstanceKey::new_v4(),
            None,
            tx,
            Bootstrap::ListOnly,
            dummy_permissions(),
            None,
            None,
        );

        let settled = tokio::time::timeout(std::time::Duration::from_secs(15), async {
            loop {
                match rx.recv().await {
                    Ok(InstanceEvent::State {
                        state: InstanceState::Error,
                        ..
                    })
                    | Ok(InstanceEvent::State {
                        state: InstanceState::Ended,
                        ..
                    }) => return Ok::<(), ()>(()),
                    Ok(_) => continue,
                    Err(_) => return Err(()),
                }
            }
        })
        .await
        .expect("actor settles");
        assert!(settled.is_ok());

        drop(handle);
    }

    /// Regression for the "LLM responses don't show" bug: awaiting a
    /// long-running request inline inside the select! arm blocks the
    /// event-forwarding arm on the same loop, starving transcript +
    /// permission-request fanout. The fix detaches the request into
    /// its own `tokio::spawn` so the loop keeps polling
    /// `client_events_rx`. This test models the select!'s contract on
    /// pure channels (no real ACP connection needed).
    #[tokio::test(start_paused = true)]
    async fn select_loop_pumps_events_while_request_outstanding() {
        use tokio::sync::{mpsc, oneshot};

        enum Cmd {
            Request { reply: oneshot::Sender<()> },
        }

        let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<Cmd>();
        let (evt_tx, mut evt_rx) = mpsc::unbounded_channel::<&'static str>();
        let (observed_tx, mut observed_rx) = mpsc::unbounded_channel::<&'static str>();

        let loop_handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    cmd = cmd_rx.recv() => {
                        let Some(cmd) = cmd else { break };
                        match cmd {
                            // Same shape as the fixed `Prompt` arm: spawn, do not await.
                            Cmd::Request { reply } => {
                                tokio::spawn(async move {
                                    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                                    let _ = reply.send(());
                                });
                            }
                        }
                    }
                    evt = evt_rx.recv() => {
                        let Some(evt) = evt else { break };
                        let _ = observed_tx.send(evt);
                    }
                }
            }
        });

        let (reply_tx, reply_rx) = oneshot::channel();
        cmd_tx.send(Cmd::Request { reply: reply_tx }).unwrap();
        evt_tx.send("mid-flight").unwrap();

        let observed = tokio::time::timeout(std::time::Duration::from_millis(50), observed_rx.recv())
            .await
            .expect("event forwarded while request outstanding")
            .expect("channel open");
        assert_eq!(observed, "mid-flight");

        tokio::time::advance(std::time::Duration::from_secs(11)).await;
        let _ = reply_rx.await;
        drop(cmd_tx);
        drop(evt_tx);
        let _ = loop_handle.await;
    }

    /// `Bootstrap::Resume` against a child that dies before responding
    /// never leaks a partial session — the actor funnels through
    /// `InstanceState::Error`. The capability gate is a pre-connection
    /// check; integration coverage lives against the mock agent.
    #[tokio::test(flavor = "multi_thread")]
    async fn resume_against_dead_child_reports_error() {
        let (tx, mut rx) = broadcast::channel(8);
        let handle = AcpInstance::start(
            dummy_resolved("ded-resume"),
            crate::adapters::InstanceKey::new_v4(),
            None,
            tx,
            Bootstrap::Resume("00000000-0000-0000-0000-000000000000".into()),
            dummy_permissions(),
            None,
            None,
        );

        let settled = tokio::time::timeout(std::time::Duration::from_secs(15), async {
            loop {
                match rx.recv().await {
                    Ok(InstanceEvent::State {
                        state: InstanceState::Error,
                        ..
                    })
                    | Ok(InstanceEvent::State {
                        state: InstanceState::Ended,
                        ..
                    }) => return Ok::<(), ()>(()),
                    Ok(_) => continue,
                    Err(_) => return Err(()),
                }
            }
        })
        .await
        .expect("actor settles");
        assert!(settled.is_ok());

        drop(handle);
    }
}
