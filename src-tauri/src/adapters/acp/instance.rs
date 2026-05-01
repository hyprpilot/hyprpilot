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
    AudioContent, BlobResourceContents, CancelNotification, ClientCapabilities, ContentBlock, EmbeddedResource,
    EmbeddedResourceResource, FileSystemCapabilities, ImageContent, InitializeRequest, ListSessionsRequest,
    ListSessionsResponse, LoadSessionRequest, ModelId, NewSessionRequest, PromptRequest, ProtocolVersion, SessionId,
    SessionModeId, SetSessionModeRequest, SetSessionModelRequest, TextContent, TextResourceContents,
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
    /// Switch the active operational mode (e.g. claude-code's
    /// `plan` / `edit`). Sends ACP `session/set_mode` and updates the
    /// per-instance metadata so the next `InstanceMeta` event carries
    /// the new value.
    SetMode {
        mode_id: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Switch the active model on a live session. ACP gates this
    /// behind the `unstable_session_model` feature; our dependency
    /// has it enabled via `["unstable"]`.
    SetModel {
        model_id: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Ask the agent for its persisted session index. Works in any
    /// bootstrap mode — the actor is always past `initialize` by the
    /// time it processes commands.
    ListSessions {
        cwd: Option<std::path::PathBuf>,
        reply: oneshot::Sender<Result<ListSessionsResponse, String>>,
    },
    /// Read the actor's current cached metadata (cwd, current
    /// mode/model id, advertised mode/model lists). Powers the
    /// `instance_meta` Tauri command used by the palette pickers —
    /// the palette ALWAYS routes through this snapshot rather than
    /// reading the UI-side `useSessionInfo` cache, so a stale UI
    /// state can't desync the picker from the daemon's authoritative
    /// view.
    MetaSnapshot {
        reply: oneshot::Sender<MetaSnapshot>,
    },
    /// Shutdown hook — stops the actor after the current prompt
    /// (or immediately if idle). Reply carries the final state.
    Shutdown {
        reply: oneshot::Sender<()>,
    },
}

/// Snapshot of the per-instance metadata the daemon caches off
/// `NewSessionResponse` / `LoadSessionResponse` / `set_mode` /
/// `set_model` replies. Read-only view; identical to the payload
/// the `acp:instance-meta` Tauri event carries, minus identity
/// fields the caller already knows.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetaSnapshot {
    pub session_id: Option<String>,
    pub cwd: String,
    pub current_mode_id: Option<String>,
    pub current_model_id: Option<String>,
    pub available_modes: Vec<crate::adapters::SessionModeInfo>,
    pub available_models: Vec<crate::adapters::SessionModelInfo>,
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
/// Outcome of mapping one ACP `SessionUpdate` payload. Most variants
/// land in the transcript; metadata updates (`session_info_update`,
/// `current_mode_update`) are routed to dedicated `InstanceEvent`
/// variants instead since they aren't transcript content.
pub(crate) enum MappedUpdate {
    Transcript(crate::adapters::TranscriptItem),
    SessionInfo {
        title: Option<String>,
        updated_at: Option<String>,
    },
    CurrentMode {
        current_mode_id: String,
    },
    AvailableCommands {
        commands: Vec<crate::completion::source::commands::CommandSummary>,
    },
}

pub(crate) fn map_session_update(update: serde_json::Value) -> MappedUpdate {
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

    match kind.as_str() {
        "user_message_chunk" => MappedUpdate::Transcript(TranscriptItem::UserText {
            text: chunk_text(&update),
        }),
        "agent_message_chunk" => MappedUpdate::Transcript(TranscriptItem::AgentText {
            text: chunk_text(&update),
        }),
        "agent_thought_chunk" => MappedUpdate::Transcript(TranscriptItem::AgentThought {
            text: chunk_text(&update),
        }),
        "session_info_update" => MappedUpdate::SessionInfo {
            title: update.get("title").and_then(|v| v.as_str()).map(str::to_string),
            updated_at: update.get("updatedAt").and_then(|v| v.as_str()).map(str::to_string),
        },
        "current_mode_update" => MappedUpdate::CurrentMode {
            current_mode_id: update
                .get("currentModeId")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        },
        "available_commands_update" => {
            use crate::completion::source::commands::CommandSummary;
            let commands = update
                .get("availableCommands")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|entry| {
                            let name = entry.get("name").and_then(|v| v.as_str())?.to_string();
                            let description = entry.get("description").and_then(|v| v.as_str()).map(str::to_string);
                            Some(CommandSummary { name, description })
                        })
                        .collect()
                })
                .unwrap_or_default();
            MappedUpdate::AvailableCommands { commands }
        }
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
            let raw_input = update.get("rawInput").cloned();
            let content = update.get("content").map(parse_content).unwrap_or_default();
            // Surface the full tool-call payload at debug. The
            // dedicated `acp::tool_call` target lets developers crank
            // the level for this subsystem alone via
            // `RUST_LOG=acp::tool_call=debug,info` when they need to
            // see a wire shape for a new formatter without drowning
            // in the rest of the daemon's debug stream.
            tracing::debug!(
                target: "acp::tool_call",
                id = %id,
                kind = %tool_kind,
                title = %title,
                state = ?state,
                raw_input = ?raw_input,
                content_blocks = content.len(),
                "acp::instance: tool_call payload (formatter input)"
            );
            MappedUpdate::Transcript(TranscriptItem::ToolCall(ToolCallRecord {
                id,
                tool_kind,
                title,
                state,
                raw_input,
                content,
            }))
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
            let raw_input = update.get("rawInput").cloned();
            let content = update.get("content").map(parse_content).unwrap_or_default();
            tracing::debug!(
                target: "acp::tool_call",
                id = %id,
                kind = ?tool_kind,
                title = ?title,
                state = ?state,
                raw_input = ?raw_input,
                content_blocks = content.len(),
                "acp::instance: tool_call_update payload (formatter input)"
            );
            MappedUpdate::Transcript(TranscriptItem::ToolCallUpdate(ToolCallUpdateRecord {
                id,
                tool_kind,
                title,
                state,
                raw_input,
                content,
            }))
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
            MappedUpdate::Transcript(TranscriptItem::Plan(PlanRecord { steps }))
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
            MappedUpdate::Transcript(TranscriptItem::PermissionRequest(PermissionRequestRecord {
                request_id,
                tool,
                tool_kind,
                args,
                options,
            }))
        }
        _ => MappedUpdate::Transcript(TranscriptItem::Unknown {
            wire_kind: kind,
            payload: update,
        }),
    }
}

/// Build the ACP `Vec<ContentBlock>` payload for one user turn.
///
/// Attachments dispatch onto an ACP `ContentBlock` variant purely
/// from MIME type — no per-attachment "is this an image?" flag
/// hardcoded into the type. The caller fills in `data` (base64 for
/// binary content) or `body` (text), tags the MIME, and the encoder
/// picks the right wire shape:
///
/// - `image/*` (with base64 `data`)  → `ContentBlock::Image`
/// - `audio/*` (with base64 `data`)  → `ContentBlock::Audio`
/// - any text-shaped MIME            → `ContentBlock::Resource(TextResourceContents)`
///   (text/markdown, text/plain, application/json, application/xml,
///   application/x-yaml, etc. — anything where the body is meaningful
///   as a UTF-8 string)
/// - everything else (with `data`)   → `ContentBlock::Resource(BlobResourceContents)`
///   (PDFs, archives, binaries — base64-encoded blob the agent can
///   reference by URI)
///
/// Falls back to a text resource when no `data` is present and the
/// MIME isn't image/audio — covers the legacy skill-attachment path
/// where `body` is a markdown string and `mime` is unset.
///
/// Prose text always lands last per the convention documented on
/// `UserTurnInput`: agents read context before instructions.
pub(crate) fn build_prompt_blocks(text: &str, attachments: &[Attachment]) -> Vec<ContentBlock> {
    let mut blocks: Vec<ContentBlock> = Vec::with_capacity(attachments.len() + 1);
    for att in attachments {
        blocks.push(attachment_to_block(att));
    }
    blocks.push(ContentBlock::Text(TextContent::new(text.to_owned())));
    blocks
}

/// Project a single attachment onto the matching ACP wire variant
/// based on its MIME type. Pure function — no I/O.
fn attachment_to_block(att: &Attachment) -> ContentBlock {
    let mime = att.mime_type();
    match mime_category(&mime) {
        MimeCategory::Image => {
            let mut img = ImageContent::new(att.data.clone().unwrap_or_default(), mime);
            img.uri = Some(att.file_uri());
            ContentBlock::Image(img)
        }
        // ACP's `AudioContent` carries no `uri` field (unlike
        // `ImageContent`), so the file_uri is intentionally dropped.
        MimeCategory::Audio => ContentBlock::Audio(AudioContent::new(att.data.clone().unwrap_or_default(), mime)),
        MimeCategory::Text => {
            let mut tr = TextResourceContents::new(att.body.clone(), att.file_uri());
            tr.mime_type = Some(mime);
            ContentBlock::Resource(EmbeddedResource::new(EmbeddedResourceResource::TextResourceContents(
                tr,
            )))
        }
        MimeCategory::Blob => {
            let mut blob = BlobResourceContents::new(att.data.clone().unwrap_or_default(), att.file_uri());
            blob.mime_type = Some(mime);
            ContentBlock::Resource(EmbeddedResource::new(EmbeddedResourceResource::BlobResourceContents(
                blob,
            )))
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum MimeCategory {
    Image,
    Audio,
    /// Text-shaped: encodes through `TextResourceContents` using the
    /// attachment's `body` field. Covers `text/*` plus the structured
    /// formats agents commonly reason over (`application/json`,
    /// `application/xml`, `application/x-yaml`, `application/toml`).
    Text,
    /// Catch-all for anything binary that's not image/audio — PDFs,
    /// archives, octets. Encodes through `BlobResourceContents` with
    /// the base64 payload from the attachment's `data` field.
    Blob,
}

fn mime_category(mime: &str) -> MimeCategory {
    if mime.starts_with("image/") {
        return MimeCategory::Image;
    }
    if mime.starts_with("audio/") {
        return MimeCategory::Audio;
    }
    if mime.starts_with("text/")
        || mime == "application/json"
        || mime == "application/xml"
        || mime == "application/x-yaml"
        || mime == "application/yaml"
        || mime == "application/toml"
        || mime == "application/x-toml"
        || mime == "application/javascript"
        || mime == "application/typescript"
    {
        return MimeCategory::Text;
    }
    MimeCategory::Blob
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

    /// Send the actor a `SetMode` command and await the agent's
    /// `session/set_mode` reply. Surfacing errors as `String` matches
    /// the rest of the actor's reply shape (mapped into `RpcError`
    /// upstream by `AcpAdapter::set_session_mode`).
    pub async fn set_mode(&self, mode_id: String) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        if self.cmd_tx.send(InstanceCommand::SetMode { mode_id, reply }).is_err() {
            return Err("instance actor closed".into());
        }
        rx.await.map_err(|e| e.to_string())?
    }

    /// Send the actor a `SetModel` command and await the agent's
    /// `session/set_model` reply. ACP gates this method behind
    /// `unstable_session_model`; our crate enables it via the
    /// `["unstable"]` umbrella feature on `agent-client-protocol`.
    pub async fn set_model(&self, model_id: String) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        if self.cmd_tx.send(InstanceCommand::SetModel { model_id, reply }).is_err() {
            return Err("instance actor closed".into());
        }
        rx.await.map_err(|e| e.to_string())?
    }

    /// Snapshot the actor's per-instance metadata (cwd, current
    /// mode/model id, advertised lists). Powers the `instance_meta`
    /// Tauri command — every palette open routes through here so
    /// the picker reads the daemon's authoritative cache, not a
    /// UI-side mirror of past `acp:instance-meta` events.
    pub async fn meta_snapshot(&self) -> Result<MetaSnapshot, String> {
        let (reply, rx) = oneshot::channel();
        if self.cmd_tx.send(InstanceCommand::MetaSnapshot { reply }).is_err() {
            return Err("instance actor closed".into());
        }
        rx.await.map_err(|e| e.to_string())
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
        commands_cache: Option<crate::completion::source::commands::CommandsCache>,
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
            commands_cache,
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
    commands_cache: Option<crate::completion::source::commands::CommandsCache>,
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
    let resolved_mode = resolved.mode.clone();

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

        // Per-instance metadata snapshot. The daemon emits this on
        // `InstanceEvent::InstanceMeta` because claude-code-acp doesn't
        // proactively send `SessionInfoUpdate` / `CurrentModeUpdate`
        // notifications — the UI would otherwise never see cwd / mode
        // / model values.
        let cwd_str = cwd.display().to_string();
        let current_mode_meta: Arc<tokio::sync::RwLock<Option<String>>> =
            Arc::new(tokio::sync::RwLock::new(resolved_mode.clone()));
        let current_model_meta: Arc<tokio::sync::RwLock<Option<String>>> =
            Arc::new(tokio::sync::RwLock::new(cfg.model.clone()));
        let available_modes_meta: Arc<tokio::sync::RwLock<Vec<crate::adapters::SessionModeInfo>>> =
            Arc::new(tokio::sync::RwLock::new(Vec::new()));
        let available_models_meta: Arc<tokio::sync::RwLock<Vec<crate::adapters::SessionModelInfo>>> =
            Arc::new(tokio::sync::RwLock::new(Vec::new()));

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
                // Pull `currentModeId` + `availableModes` off the
                // `NewSessionResponse.modes` field — ACP's only
                // emission of this list (no streaming variant exists).
                if let Some(modes) = &new_session.modes {
                    let advertised: Vec<crate::adapters::SessionModeInfo> = modes
                        .available_modes
                        .iter()
                        .map(|m| crate::adapters::SessionModeInfo {
                            id: m.id.0.to_string(),
                            name: m.name.clone(),
                            description: m.description.clone(),
                        })
                        .collect();
                    *available_modes_meta.write().await = advertised;
                    *current_mode_meta.write().await = Some(modes.current_mode_id.0.to_string());
                }
                // Same shape as modes, gated by ACP's `unstable_session_model`
                // feature (our crate enables `["unstable"]`). claude-code-acp
                // populates this with the agent's advertised model list +
                // current selection; without reading it here the picker's
                // `availableModels` stays empty even though the actor knows
                // how to flip via `SetSessionModelRequest`.
                if let Some(models) = &new_session.models {
                    let advertised: Vec<crate::adapters::SessionModelInfo> = models
                        .available_models
                        .iter()
                        .map(|m| crate::adapters::SessionModelInfo {
                            id: m.model_id.0.to_string(),
                            name: m.name.clone(),
                            description: m.description.clone(),
                        })
                        .collect();
                    *available_models_meta.write().await = advertised;
                    *current_model_meta.write().await = Some(models.current_model_id.0.to_string());
                }
                let _ = events_tx_notif.send(InstanceEvent::State {
                    agent_id: agent_id_notif.clone(),
                    instance_id: instance_id_notif.clone(),
                    session_id: Some(sid.0.to_string()),
                    state: InstanceState::Running,
                });
                let _ = events_tx_notif.send(InstanceEvent::InstanceMeta {
                    agent_id: agent_id_notif.clone(),
                    instance_id: instance_id_notif.clone(),
                    session_id: Some(sid.0.to_string()),
                    cwd: cwd_str.clone(),
                    current_mode_id: current_mode_meta.read().await.clone(),
                    current_model_id: current_model_meta.read().await.clone(),
                    available_modes: available_modes_meta.read().await.clone(),
                    available_models: available_models_meta.read().await.clone(),
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
                let load_resp = match connection
                    .send_request(LoadSessionRequest::new(sid.clone(), cwd.clone()))
                    .block_task()
                    .await
                {
                    Ok(resp) => resp,
                    Err(err) => {
                        warn!(agent = %agent_id_notif, %err, "acp::instance: load_session failed");
                        let _ = events_tx_notif.send(InstanceEvent::State {
                            agent_id: agent_id_notif.clone(),
                            instance_id: instance_id_notif.clone(),
                            session_id: Some(sid.0.to_string()),
                            state: InstanceState::Error,
                        });
                        return Err(err);
                    }
                };
                info!(
                    agent = %agent_id_notif,
                    instance = %instance_id_notif,
                    session = %sid,
                    "acp::instance: session/load accepted"
                );
                // Mirror the Fresh path's `NewSessionResponse.modes/models`
                // read against `LoadSessionResponse`. Without this the
                // resumed instance's mode/model pickers stay empty —
                // the agent advertises both lists in the load response,
                // but we'd been discarding it. Same shapes, same logic
                // as the Fresh arm above.
                if let Some(modes) = &load_resp.modes {
                    let advertised: Vec<crate::adapters::SessionModeInfo> = modes
                        .available_modes
                        .iter()
                        .map(|m| crate::adapters::SessionModeInfo {
                            id: m.id.0.to_string(),
                            name: m.name.clone(),
                            description: m.description.clone(),
                        })
                        .collect();
                    *available_modes_meta.write().await = advertised;
                    *current_mode_meta.write().await = Some(modes.current_mode_id.0.to_string());
                }
                if let Some(models) = &load_resp.models {
                    let advertised: Vec<crate::adapters::SessionModelInfo> = models
                        .available_models
                        .iter()
                        .map(|m| crate::adapters::SessionModelInfo {
                            id: m.model_id.0.to_string(),
                            name: m.name.clone(),
                            description: m.description.clone(),
                        })
                        .collect();
                    *available_models_meta.write().await = advertised;
                    *current_model_meta.write().await = Some(models.current_model_id.0.to_string());
                }
                // Suspended sessions can resume with a half-finished
                // turn — pending tool call awaiting permission, agent
                // mid-stream, etc. The replay surfaces those states
                // in the transcript, but the agent server-side might
                // still treat the original turn as "in flight",
                // refusing fresh prompts until something resolves it.
                // Send a CancelNotification right after the load
                // accepts so any inherited in-flight state collapses
                // before the user types. Soft-fails: if the session
                // is already idle, the agent treats it as a no-op.
                if let Err(err) = connection.send_notification(CancelNotification::new(sid.clone())) {
                    debug!(
                        agent = %agent_id_notif,
                        session = %sid,
                        %err,
                        "acp::instance: post-load cancel notification failed (non-fatal)"
                    );
                }
                let _ = events_tx_notif.send(InstanceEvent::State {
                    agent_id: agent_id_notif.clone(),
                    instance_id: instance_id_notif.clone(),
                    session_id: Some(sid.0.to_string()),
                    state: InstanceState::Running,
                });
                let _ = events_tx_notif.send(InstanceEvent::InstanceMeta {
                    agent_id: agent_id_notif.clone(),
                    instance_id: instance_id_notif.clone(),
                    session_id: Some(sid.0.to_string()),
                    cwd: cwd_str.clone(),
                    current_mode_id: current_mode_meta.read().await.clone(),
                    current_model_id: current_model_meta.read().await.clone(),
                    available_modes: available_modes_meta.read().await.clone(),
                    available_models: available_models_meta.read().await.clone(),
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
                            let cwd_done = cwd_str.clone();
                            let current_mode_done = current_mode_meta.clone();
                            let current_model_done = current_model_meta.clone();
                            let available_modes_done = available_modes_meta.clone();
                            let available_models_done = available_models_meta.clone();
                            tokio::spawn(async move {
                                let res = conn
                                    .send_request(PromptRequest::new(sid.clone(), blocks))
                                    .block_task()
                                    .await;
                                let (stop_reason, error_msg, mapped) = match res {
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
                                        (stop, None, Ok(()))
                                    }
                                    Err(err) => {
                                        warn!(
                                            agent = %agent_log,
                                            session = %session_log,
                                            turn = %turn_id_done,
                                            %err,
                                            "acp::instance: turn ended with error"
                                        );
                                        let msg = err.to_string();
                                        (None, Some(msg.clone()), Err(msg))
                                    }
                                };
                                {
                                    let mut slot = current_turn_done.write().await;
                                    if slot.as_deref() == Some(turn_id_done.as_str()) {
                                        *slot = None;
                                    }
                                }
                                let _ = events_tx_done.send(InstanceEvent::TurnEnded {
                                    agent_id: agent_id_done.clone(),
                                    instance_id: instance_id_done.clone(),
                                    session_id: sid.0.to_string(),
                                    turn_id: turn_id_done,
                                    stop_reason,
                                    error: error_msg,
                                });
                                // Refresh tick after every turn end so the
                                // header chrome re-syncs even when the agent
                                // didn't push a `current_mode_update` /
                                // `session_info_update` notification this turn.
                                let _ = events_tx_done.send(InstanceEvent::InstanceMeta {
                                    agent_id: agent_id_done,
                                    instance_id: instance_id_done,
                                    session_id: Some(sid.0.to_string()),
                                    cwd: cwd_done,
                                    current_mode_id: current_mode_done.read().await.clone(),
                                    current_model_id: current_model_done.read().await.clone(),
                                    available_modes: available_modes_done.read().await.clone(),
                                    available_models: available_models_done.read().await.clone(),
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
                        InstanceCommand::SetMode { mode_id, reply } => {
                            let Some(sid) = session_id.clone() else {
                                let _ = reply.send(Err("no live session in list-only actor".into()));
                                continue;
                            };
                            info!(
                                agent = %agent_id_notif,
                                session = %sid,
                                mode_id,
                                "acp::instance: session/set_mode requested"
                            );
                            let conn = connection.clone();
                            let agent_log = agent_id_notif.clone();
                            let instance_log = instance_id_notif.clone();
                            let session_log = sid.clone();
                            let current_mode = current_mode_meta.clone();
                            let available_modes = available_modes_meta.clone();
                            let current_model_done = current_model_meta.clone();
                            let available_models_done = available_models_meta.clone();
                            let cwd_done = cwd_str.clone();
                            let events_tx_done = events_tx_notif.clone();
                            tokio::spawn(async move {
                                let req = SetSessionModeRequest::new(session_log.clone(), SessionModeId::from(std::sync::Arc::<str>::from(mode_id.clone())));
                                let res = conn
                                    .send_request(req)
                                    .block_task()
                                    .await
                                    .map_err(|e| e.to_string());
                                if res.is_ok() {
                                    *current_mode.write().await = Some(mode_id.clone());
                                    // Refresh InstanceMeta so the header
                                    // picks up the new mode without
                                    // waiting for an agent-pushed
                                    // current_mode_update.
                                    let _ = events_tx_done.send(InstanceEvent::InstanceMeta {
                                        agent_id: agent_log,
                                        instance_id: instance_log,
                                        session_id: Some(session_log.0.to_string()),
                                        cwd: cwd_done,
                                        current_mode_id: Some(mode_id),
                                        current_model_id: current_model_done.read().await.clone(),
                                        available_modes: available_modes.read().await.clone(),
                                        available_models: available_models_done.read().await.clone(),
                                    });
                                }
                                let _ = reply.send(res.map(|_| ()));
                            });
                        }
                        InstanceCommand::SetModel { model_id, reply } => {
                            let Some(sid) = session_id.clone() else {
                                let _ = reply.send(Err("no live session in list-only actor".into()));
                                continue;
                            };
                            info!(
                                agent = %agent_id_notif,
                                session = %sid,
                                model_id,
                                "acp::instance: session/set_model requested"
                            );
                            let conn = connection.clone();
                            let agent_log = agent_id_notif.clone();
                            let instance_log = instance_id_notif.clone();
                            let session_log = sid.clone();
                            let current_mode_done = current_mode_meta.clone();
                            let current_model_done = current_model_meta.clone();
                            let available_modes_done = available_modes_meta.clone();
                            let available_models_done = available_models_meta.clone();
                            let cwd_done = cwd_str.clone();
                            let events_tx_done = events_tx_notif.clone();
                            tokio::spawn(async move {
                                let req = SetSessionModelRequest::new(session_log.clone(), ModelId::from(std::sync::Arc::<str>::from(model_id.clone())));
                                let res = conn
                                    .send_request(req)
                                    .block_task()
                                    .await
                                    .map_err(|e| e.to_string());
                                if res.is_ok() {
                                    *current_model_done.write().await = Some(model_id.clone());
                                    let _ = events_tx_done.send(InstanceEvent::InstanceMeta {
                                        agent_id: agent_log,
                                        instance_id: instance_log,
                                        session_id: Some(session_log.0.to_string()),
                                        cwd: cwd_done,
                                        current_mode_id: current_mode_done.read().await.clone(),
                                        current_model_id: Some(model_id),
                                        available_modes: available_modes_done.read().await.clone(),
                                        available_models: available_models_done.read().await.clone(),
                                    });
                                }
                                let _ = reply.send(res.map(|_| ()));
                            });
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
                        InstanceCommand::MetaSnapshot { reply } => {
                            // Direct read of the per-instance Arc-cached
                            // metadata. Fast — no agent roundtrip — but
                            // returns the freshest state the daemon has
                            // (updated on session/new, session/load,
                            // set_mode, set_model, every TurnEnded).
                            let snap = MetaSnapshot {
                                session_id: session_id.as_ref().map(|s| s.0.to_string()),
                                cwd: cwd_str.clone(),
                                current_mode_id: current_mode_meta.read().await.clone(),
                                current_model_id: current_model_meta.read().await.clone(),
                                available_modes: available_modes_meta.read().await.clone(),
                                available_models: available_models_meta.read().await.clone(),
                            };
                            let _ = reply.send(snap);
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
                            let mapped = map_session_update(update);
                            let turn_id = current_turn_id.read().await.clone();
                            let evt: Option<InstanceEvent> = match mapped {
                                MappedUpdate::Transcript(item) => Some(InstanceEvent::Transcript {
                                    agent_id: agent_id_notif.clone(),
                                    instance_id: instance_id_notif.clone(),
                                    session_id: sid,
                                    turn_id,
                                    item,
                                }),
                                MappedUpdate::SessionInfo { title, updated_at } => Some(InstanceEvent::SessionInfoUpdate {
                                    agent_id: agent_id_notif.clone(),
                                    instance_id: instance_id_notif.clone(),
                                    session_id: sid,
                                    title,
                                    updated_at,
                                }),
                                MappedUpdate::CurrentMode { current_mode_id } => {
                                    *current_mode_meta.write().await = Some(current_mode_id.clone());
                                    Some(InstanceEvent::CurrentModeUpdate {
                                        agent_id: agent_id_notif.clone(),
                                        instance_id: instance_id_notif.clone(),
                                        session_id: sid,
                                        current_mode_id,
                                    })
                                }
                                MappedUpdate::AvailableCommands { commands } => {
                                    if let Some(cache) = commands_cache.as_ref() {
                                        match cache.write() {
                                            Ok(mut guard) => {
                                                debug!(
                                                    instance = %instance_id_notif,
                                                    count = commands.len(),
                                                    "acp::instance: available_commands_update — refreshing autocomplete cache"
                                                );
                                                *guard = commands;
                                            }
                                            Err(err) => {
                                                tracing::warn!(%err, "acp::instance: commands_cache lock poisoned");
                                            }
                                        }
                                    }
                                    None
                                }
                            };
                            if let Some(evt) = evt {
                                let _ = events_tx_notif.send(evt);
                            }
                        }
                        ClientEvent::PermissionRequested {
                            session_id: sid,
                            request_id,
                            tool,
                            kind,
                            args,
                            raw_input,
                            content_text,
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
                                raw_input,
                                content_text,
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
            data: None,
            mime: None,
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
    fn build_prompt_blocks_dispatches_image_audio_blob_purely_by_mime() {
        let img = Attachment {
            slug: "shot".into(),
            path: PathBuf::from("/tmp/shot.png"),
            body: String::new(),
            title: None,
            data: Some("BASE64IMG".into()),
            mime: Some("image/png".into()),
        };
        let audio = Attachment {
            slug: "clip".into(),
            path: PathBuf::from("/tmp/clip.wav"),
            body: String::new(),
            title: None,
            data: Some("BASE64AUDIO".into()),
            mime: Some("audio/wav".into()),
        };
        let pdf = Attachment {
            slug: "doc".into(),
            path: PathBuf::from("/tmp/doc.pdf"),
            body: String::new(),
            title: None,
            data: Some("BASE64PDF".into()),
            mime: Some("application/pdf".into()),
        };
        let yaml = Attachment {
            slug: "cfg".into(),
            path: PathBuf::from("/tmp/cfg.yaml"),
            body: "name: hyprpilot".into(),
            title: None,
            data: None,
            mime: Some("application/x-yaml".into()),
        };
        let blocks = build_prompt_blocks("text", &[img, audio, pdf, yaml]);
        assert_eq!(blocks.len(), 5, "4 attachments + 1 text");
        match &blocks[0] {
            ContentBlock::Image(i) => {
                assert_eq!(i.mime_type, "image/png");
                assert_eq!(i.data, "BASE64IMG");
            }
            other => panic!("expected image, got {other:?}"),
        }
        match &blocks[1] {
            ContentBlock::Audio(a) => {
                assert_eq!(a.mime_type, "audio/wav");
                assert_eq!(a.data, "BASE64AUDIO");
            }
            other => panic!("expected audio, got {other:?}"),
        }
        let ContentBlock::Resource(pdf_res) = &blocks[2] else {
            panic!("expected resource for PDF")
        };
        let EmbeddedResourceResource::BlobResourceContents(blob) = &pdf_res.resource else {
            panic!("PDF must encode as blob, not text")
        };
        assert_eq!(blob.blob, "BASE64PDF");
        assert_eq!(blob.mime_type.as_deref(), Some("application/pdf"));
        let ContentBlock::Resource(yaml_res) = &blocks[3] else {
            panic!("expected resource for yaml")
        };
        let EmbeddedResourceResource::TextResourceContents(tr) = &yaml_res.resource else {
            panic!("yaml must encode as text resource")
        };
        assert_eq!(tr.text, "name: hyprpilot");
        assert_eq!(tr.mime_type.as_deref(), Some("application/x-yaml"));
    }

    #[test]
    fn mime_category_classifies_known_types() {
        assert_eq!(mime_category("image/png"), MimeCategory::Image);
        assert_eq!(mime_category("image/svg+xml"), MimeCategory::Image);
        assert_eq!(mime_category("audio/mp3"), MimeCategory::Audio);
        assert_eq!(mime_category("text/plain"), MimeCategory::Text);
        assert_eq!(mime_category("text/markdown"), MimeCategory::Text);
        assert_eq!(mime_category("application/json"), MimeCategory::Text);
        assert_eq!(mime_category("application/x-yaml"), MimeCategory::Text);
        assert_eq!(mime_category("application/pdf"), MimeCategory::Blob);
        assert_eq!(mime_category("application/octet-stream"), MimeCategory::Blob);
    }

    #[test]
    fn build_prompt_blocks_preserves_attachment_order() {
        let a = Attachment {
            slug: "a".into(),
            path: PathBuf::from("/tmp/a/SKILL.md"),
            body: "A".into(),
            title: None,
            data: None,
            mime: None,
        };
        let b = Attachment {
            slug: "b".into(),
            path: PathBuf::from("/tmp/b/SKILL.md"),
            body: "B".into(),
            title: None,
            data: None,
            mime: None,
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
