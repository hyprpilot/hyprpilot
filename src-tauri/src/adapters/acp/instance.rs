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
use crate::config::AgentConfig;
use crate::tools::{TerminalToolEventKind, TerminalToolStream};

/// How long the registry waits for the actor to ack a `Shutdown`
/// command before dropping the handle.
const SHUTDOWN_ACK_TIMEOUT: Duration = Duration::from_secs(2);

/// RAII handle for one open `session/prompt` turn. Constructor emits
/// `TurnStarted` + claims the `current_turn_id` slot; `complete(...)`
/// emits `TurnEnded` with the agent-supplied stop reason / error;
/// `Drop` is the leak fallback — when the spawned prompt task panics,
/// the transport closes mid-turn, or the actor unwinds before
/// `complete` runs, the guard synthesises `TurnEnded { stop_reason:
/// "cancelled" }` and frees the slot.
///
/// The slot still mediates ownership across the actor + the spawn
/// future: a concurrent `Cancel` handler atomically takes the slot
/// and emits its own `TurnEnded { stop_reason: "cancelled" }`.
/// `complete` (and `Drop`) re-check ownership before emitting, so a
/// raced cancel doesn't double-fire.
struct TurnGuard {
    turn_id: String,
    instance_id: String,
    agent_id: String,
    session_id: String,
    events_tx: broadcast::Sender<InstanceEvent>,
    current_turn_id: Arc<tokio::sync::RwLock<Option<String>>>,
    completed: bool,
}

impl TurnGuard {
    async fn new(
        turn_id: String,
        agent_id: String,
        instance_id: String,
        session_id: String,
        events_tx: broadcast::Sender<InstanceEvent>,
        current_turn_id: Arc<tokio::sync::RwLock<Option<String>>>,
    ) -> Self {
        *current_turn_id.write().await = Some(turn_id.clone());
        let _ = events_tx.send(InstanceEvent::TurnStarted {
            agent_id: agent_id.clone(),
            instance_id: instance_id.clone(),
            session_id: session_id.clone(),
            turn_id: turn_id.clone(),
        });
        Self {
            turn_id,
            instance_id,
            agent_id,
            session_id,
            events_tx,
            current_turn_id,
            completed: false,
        }
    }

    /// Returns true when this guard still owned the slot at emit time
    /// (so the caller knows whether to follow up with the per-turn
    /// `InstanceMeta` refresh — only fires alongside an emit).
    async fn complete(mut self, stop_reason: Option<String>, error: Option<String>) -> bool {
        self.completed = true;
        let still_owned = {
            let mut slot = self.current_turn_id.write().await;
            if slot.as_deref() == Some(self.turn_id.as_str()) {
                *slot = None;
                true
            } else {
                false
            }
        };
        if !still_owned {
            return false;
        }
        let _ = self.events_tx.send(InstanceEvent::TurnEnded {
            agent_id: self.agent_id.clone(),
            instance_id: self.instance_id.clone(),
            session_id: self.session_id.clone(),
            turn_id: self.turn_id.clone(),
            stop_reason,
            error,
        });
        true
    }
}

impl Drop for TurnGuard {
    fn drop(&mut self) {
        if self.completed {
            return;
        }
        // Best-effort: if a concurrent task holds the slot we let it
        // win (it'll synthesise its own TurnEnded). try_write avoids
        // blocking + the awkward "Drop in async context" trap.
        let mut slot = match self.current_turn_id.try_write() {
            Ok(g) => g,
            Err(_) => return,
        };
        if slot.as_deref() != Some(self.turn_id.as_str()) {
            return;
        }
        *slot = None;
        let _ = self.events_tx.send(InstanceEvent::TurnEnded {
            agent_id: self.agent_id.clone(),
            instance_id: self.instance_id.clone(),
            session_id: self.session_id.clone(),
            turn_id: self.turn_id.clone(),
            stop_reason: Some("cancelled".to_string()),
            error: None,
        });
    }
}

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
    /// Switch a generic session config option — ACP's
    /// `session/set_config_option`. Carries
    /// `thought_level` (a reserved spec category for reasoning depth),
    /// `mode` / `model` (when the agent surfaces them via
    /// `configOptions` instead of dedicated mode / model surfaces), AND
    /// every vendor-specific category whose id starts with `_` (per
    /// spec: `_*` ids are free for custom use). The captain picks one
    /// of the offered values via the palette; this command sends the
    /// pick to the agent + adopts the response's
    /// `configOptions: Vec<SessionConfigOption>` as the new advertised
    /// set. Independent of `set_mode` / `set_model`: those address the
    /// dedicated wire methods; this one is the generic catch-all.
    SetConfigOption {
        config_id: String,
        value: String,
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
    pub mcps_count: usize,
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
/// `Transcript` carries the full `TranscriptItem` enum (~250 bytes
/// for the largest variant: `ToolCall` with embedded `FormattedToolCall`).
/// Sibling variants (`SessionInfo`, `CurrentMode`, `AvailableCommands`)
/// are smaller; boxing the larger one would force a heap allocation
/// for every transcript chunk on the hot path. The size disparity is
/// the cost of the wire-shape simplicity.
#[allow(clippy::large_enum_variant)]
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

/// Outcome of one mapper call — the typed `MappedUpdate` plus the
/// envelope's `_meta` (vendor-specific extension data). `_meta` rides
/// alongside on `InstanceEvent::Transcript`; today no UI consumer
/// reads it, but the pass-through is wired so future per-vendor UI
/// hooks plug in without a wire change.
pub(crate) struct MappedSessionUpdate {
    pub mapped: MappedUpdate,
    pub meta: Option<serde_json::Value>,
}

/// Per-id running tool-call state — feeds the formatter on every
/// `tool_call_update` so the `formatted` snapshot reflects merged
/// state, not just the delta. Owned by the per-instance notification
/// task; cleared on session boundary (load_session swap, shutdown).
#[derive(Debug, Default, Clone)]
pub(crate) struct RunningToolCall {
    pub wire_name: String,
    pub tool_kind: String,
    pub raw_input: Option<serde_json::Value>,
    pub content: Vec<serde_json::Value>,
}

pub(crate) type ToolCallCache = std::collections::HashMap<String, RunningToolCall>;

fn format_running(adapter_id: &str, running: &RunningToolCall) -> crate::tools::formatter::types::FormattedToolCall {
    use crate::tools::formatter::registry::FormatterContext;
    let registry = crate::adapters::acp::formatter_registry();
    let ctx = FormatterContext {
        wire_name: running.wire_name.as_str(),
        kind: running.tool_kind.as_str(),
        raw_input: running.raw_input.as_ref(),
        adapter: adapter_id,
        content: &running.content,
    };
    registry.dispatch(&ctx)
}

pub(crate) fn map_session_update(
    update: serde_json::Value,
    tool_calls: &mut ToolCallCache,
    adapter_id: &str,
) -> MappedSessionUpdate {
    use crate::adapters::{
        Attachment, PermissionRequestRecord, PlanRecord, PlanStep, ToolCallContentItem, ToolCallRecord, ToolCallState,
        ToolCallUpdateRecord, TranscriptItem,
    };

    let kind = update
        .get("sessionUpdate")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let meta = update.get("_meta").cloned();

    fn chunk_text(update: &serde_json::Value) -> String {
        update
            .get("content")
            .and_then(|c| c.get("text"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    }

    /// Project a single agent-emitted `ContentBlock` (the chunk's
    /// `content` slot) into either `AgentText` or `AgentAttachment`.
    /// Mirrors the user-side encoder in `build_prompt_blocks` —
    /// dispatches purely on `type` and (for `resource`) the inner
    /// resource discriminator. Unknown shapes fall through to
    /// `Unknown` so the UI logs the gap without bricking the session.
    fn project_agent_chunk_content(content: &serde_json::Value) -> TranscriptItem {
        let block_type = content.get("type").and_then(|v| v.as_str()).unwrap_or("text");
        match block_type {
            "text" => TranscriptItem::AgentText {
                text: content.get("text").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            },
            "image" | "audio" => {
                let mime = content
                    .get("mimeType")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .unwrap_or_else(|| {
                        if block_type == "image" {
                            "image/png".to_string()
                        } else {
                            "audio/wav".to_string()
                        }
                    });
                let data = content.get("data").and_then(|v| v.as_str()).map(str::to_string);
                // Synthesise a slug from the type + size hash so the UI
                // can dedupe; agents don't supply identifiers for
                // streaming binaries.
                let slug = format!("agent-{block_type}-{}", data.as_deref().map(str::len).unwrap_or(0));
                TranscriptItem::AgentAttachment(Attachment {
                    slug,
                    path: std::path::PathBuf::from(format!("agent-emitted-{block_type}")),
                    body: String::new(),
                    title: content.get("title").and_then(|v| v.as_str()).map(str::to_string),
                    data,
                    mime: Some(mime),
                })
            }
            "resource_link" => {
                let uri = content.get("uri").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let name = content
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .unwrap_or_else(|| uri.clone());
                let mime = content.get("mimeType").and_then(|v| v.as_str()).map(str::to_string);
                TranscriptItem::AgentAttachment(Attachment {
                    slug: format!("agent-link-{name}"),
                    path: std::path::PathBuf::from(uri),
                    body: String::new(),
                    title: content
                        .get("title")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                        .or(Some(name)),
                    data: None,
                    mime,
                })
            }
            "resource" => {
                // `resource: { uri, text? | blob?, mimeType? }`
                let inner = content.get("resource").cloned().unwrap_or(serde_json::Value::Null);
                let uri = inner.get("uri").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let mime = inner.get("mimeType").and_then(|v| v.as_str()).map(str::to_string);
                let text = inner.get("text").and_then(|v| v.as_str()).map(str::to_string);
                let blob = inner.get("blob").and_then(|v| v.as_str()).map(str::to_string);
                TranscriptItem::AgentAttachment(Attachment {
                    slug: format!("agent-resource-{uri}"),
                    path: std::path::PathBuf::from(&uri),
                    body: text.unwrap_or_default(),
                    title: Some(uri),
                    data: blob,
                    mime,
                })
            }
            other => TranscriptItem::Unknown {
                wire_kind: format!("agent_message_chunk:{other}"),
                payload: content.clone(),
            },
        }
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

    let mapped = match kind.as_str() {
        "user_message_chunk" => MappedUpdate::Transcript(TranscriptItem::UserText {
            text: chunk_text(&update),
        }),
        "agent_message_chunk" => {
            // Per ACP `agent_message_chunk` carries one ContentBlock —
            // text in the common case, image / audio / resource /
            // resource_link in the multimodal case. Project onto the
            // text or attachment variants. UI demuxer routes either.
            let content = update.get("content").cloned().unwrap_or(serde_json::Value::Null);
            MappedUpdate::Transcript(project_agent_chunk_content(&content))
        }
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
            // Update the per-id running cache so future updates
            // re-format against merged state.
            let running = RunningToolCall {
                wire_name: title.clone(),
                tool_kind: tool_kind.clone(),
                raw_input: raw_input.clone(),
                content: update
                    .get("content")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default(),
            };
            let formatted = format_running(adapter_id, &running);
            tool_calls.insert(id.clone(), running);
            MappedUpdate::Transcript(TranscriptItem::ToolCall(ToolCallRecord {
                id,
                tool_kind,
                title,
                state,
                raw_input,
                content,
                formatted,
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
            // Merge delta into the running cache — every Some-value
            // patches; raw `content` from the wire appends. `wire_name`
            // is captured once on the initial `tool_call` and stays
            // frozen: the first observation is the tool's identity
            // ("Bash", "Read", "mcp__server__leaf"), later updates
            // re-purpose `title` as a verbose human display string
            // ("bash · ls /tmp") that would defeat per-(adapter,
            // wire_name) formatter dispatch if we let it through.
            let running = tool_calls.entry(id.clone()).or_default();
            if running.wire_name.is_empty() {
                if let Some(t) = title.as_deref() {
                    running.wire_name = t.to_string();
                }
            }
            if let Some(k) = tool_kind.as_deref() {
                running.tool_kind = k.to_string();
            }
            if let Some(rv) = raw_input.as_ref() {
                running.raw_input = Some(rv.clone());
            }
            if let Some(arr) = update.get("content").and_then(|v| v.as_array()) {
                running.content.extend(arr.iter().cloned());
            }
            let formatted = format_running(adapter_id, running);
            MappedUpdate::Transcript(TranscriptItem::ToolCallUpdate(ToolCallUpdateRecord {
                id,
                tool_kind,
                title,
                state,
                raw_input,
                content,
                formatted,
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
    };
    MappedSessionUpdate { mapped, meta }
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
    /// Captain-set addressable name. Distinct from `key` (the
    /// canonical UUID). Mutated via `AdapterRegistry::rename`;
    /// validated as a slug at the rename boundary so it's always
    /// safe to display verbatim. `None` until the captain renames.
    pub name: Arc<tokio::sync::RwLock<Option<String>>>,
}

impl AcpInstance {
    pub async fn current_session_id(&self) -> Option<String> {
        self.session_id.read().await.as_ref().map(|id| id.0.to_string())
    }

    /// Snapshot the captain-set name. Returns `None` when the captain
    /// hasn't renamed the instance yet (the auto-mint has no name).
    pub async fn current_name(&self) -> Option<String> {
        self.name.read().await.clone()
    }

    /// Overwrite the captain-set name. Caller (`AdapterRegistry::rename`)
    /// is responsible for validation + uniqueness; this is a raw write.
    pub async fn set_name(&self, name: Option<String>) {
        *self.name.write().await = name;
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

    /// Send the actor a `SetConfigOption` command — ACP's
    /// `session/set_config_option`. Generic catch-all for vendor
    /// extension knobs the agent advertises in
    /// `NewSessionResponse.configOptions`; spec-reserved categories
    /// (`mode` / `model` / `thought_level`) MAY also flow through here
    /// when the agent surfaces them on configOptions instead of the
    /// dedicated wire methods. Captain picks one of the offered values
    /// from the palette; the actor sends the request, captures the
    /// response's full `configOptions` array, and refreshes the
    /// per-instance meta cache so the next palette open sees the new
    /// state.
    pub async fn set_config_option(&self, config_id: String, value: String) -> Result<(), String> {
        let (reply, rx) = oneshot::channel();
        if self
            .cmd_tx
            .send(InstanceCommand::SetConfigOption {
                config_id,
                value,
                reply,
            })
            .is_err()
        {
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
    #[must_use]
    pub fn start(params: StartParams) -> Self {
        let StartParams {
            resolved,
            key,
            profile_id,
            events_tx,
            bootstrap,
            permissions,
            mcps,
            commands_cache,
        } = params;
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

        let instance = AcpInstance {
            key,
            agent_id: resolved.agent.id.clone(),
            profile_id,
            mode,
            cmd_tx,
            session_id: session_id.clone(),
            name: Arc::new(tokio::sync::RwLock::new(None)),
        };

        tokio::spawn(run(RunParams {
            resolved,
            instance_id,
            cmd_rx,
            events_tx,
            session_id_slot: session_id,
            bootstrap,
            permissions,
            mcps,
            commands_cache,
        }));

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
        // Same try_read pattern as session_id — the rename path uses
        // a write lock briefly, every other reader (`info()`, ctl
        // listing, UI labels) is read-only. `try_read` succeeds in
        // the steady state; on lock contention we read `None` for
        // this snapshot tick and the next event sync corrects it.
        let name = self.name.try_read().ok().and_then(|n| n.clone());
        InstanceInfo {
            id: self.key.as_string(),
            name,
            agent_id: self.agent_id.clone(),
            profile_id: self.profile_id.clone(),
            session_id,
            mode: self.mode.clone(),
        }
    }

    async fn name(&self) -> Option<String> {
        self.current_name().await
    }

    async fn set_name(&self, name: Option<String>) {
        AcpInstance::set_name(self, name).await
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
/// Params for `AcpInstance::start`. Captures everything an instance
/// actor needs at construction time; same shape funnels into `run`
/// via `RunParams`.
pub struct StartParams {
    pub resolved: ResolvedInstance,
    pub key: InstanceKey,
    pub profile_id: Option<String>,
    pub events_tx: broadcast::Sender<InstanceEvent>,
    pub bootstrap: Bootstrap,
    pub permissions: Arc<dyn PermissionController>,
    pub mcps: Option<Arc<crate::mcp::MCPsRegistry>>,
    pub commands_cache: Option<crate::completion::source::commands::CommandsCache>,
}

/// Internal `run` actor params — superset of `StartParams` with the
/// command-channel receiver + the shared session-id slot the registry
/// also reads.
struct RunParams {
    resolved: ResolvedInstance,
    instance_id: String,
    cmd_rx: mpsc::UnboundedReceiver<InstanceCommand>,
    events_tx: broadcast::Sender<InstanceEvent>,
    session_id_slot: Arc<tokio::sync::RwLock<Option<SessionId>>>,
    bootstrap: Bootstrap,
    permissions: Arc<dyn PermissionController>,
    mcps: Option<Arc<crate::mcp::MCPsRegistry>>,
    commands_cache: Option<crate::completion::source::commands::CommandsCache>,
}

/// the child process, the dispatch loop. Spawned by
/// [`AcpInstance::start`].
async fn run(params: RunParams) {
    let RunParams {
        resolved,
        instance_id,
        mut cmd_rx,
        events_tx,
        session_id_slot,
        bootstrap,
        permissions,
        mcps,
        commands_cache,
    } = params;
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
    let resolved_profile_id = resolved.profile_id.clone();

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
        mcps.clone(),
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
    // Per-instance running tool-call state — feeds the formatter on
    // every `tool_call_update` so the snapshot reflects merged state
    // (not just the delta). Lives for the actor's lifetime; cleared
    // implicitly when the actor task exits.
    let mut tool_call_cache: ToolCallCache = ToolCallCache::default();
    // Adapter id for per-vendor formatter override dispatch — the
    // `[[agents]] provider` string (acp-claude-code / acp-codex /
    // acp-opencode / acp).
    let provider_id_for_fmt: String = match cfg.provider {
        crate::config::AgentProvider::AcpClaudeCode => "acp-claude-code",
        crate::config::AgentProvider::AcpCodex => "acp-codex",
        crate::config::AgentProvider::AcpOpenCode => "acp-opencode",
        crate::config::AgentProvider::Acp => "acp",
    }
    .to_string();
    // Tracks the in-flight turn id so the notification / permission
    // arms of the dispatch loop can stamp events with it without
    // re-coordinating with the Prompt-handling task. Set when a
    // `Prompt` is accepted, cleared when the spawned `session/prompt`
    // task replies. `tokio::sync::RwLock` because the spawned task
    // crosses an `.await`; reads from the loop are non-blocking.
    let current_turn_id: Arc<tokio::sync::RwLock<Option<String>>> = Arc::new(tokio::sync::RwLock::new(None));

    // Out-of-turn agent activity (scheduled wake-ups, side-channel
    // updates) arrives without a `Prompt` envelope. We mint a
    // *synthetic* turn id on the first transcript-shape notification
    // when no real turn is open, so the chat surface still groups the
    // resulting items into a single block. The synthetic turn closes
    // when a real `session/prompt` arrives and supersedes it (handled
    // at the prompt-start site). No idle auto-close — reasoning
    // models can pause arbitrarily between updates and we'd rather
    // leave the indicator running than fire a false TurnEnded
    // mid-thought.
    let synthetic_turn_id: Arc<tokio::sync::RwLock<Option<String>>> = Arc::new(tokio::sync::RwLock::new(None));

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
        // Capability probes — `agent_capabilities.session_capabilities`
        // gates the unstable `session/resume` and `session/close`
        // surfaces (both behind `unstable_session_*` features in the
        // schema crate, both already enabled by the umbrella
        // `unstable` feature on `agent-client-protocol`). Falling
        // through to `false` when the agent doesn't advertise the
        // capability — we silently use the legacy paths
        // (LoadSession / CancelNotification).
        let resume_supported = init.agent_capabilities.session_capabilities.resume.is_some();
        let close_supported = init.agent_capabilities.session_capabilities.close.is_some();
        info!(
            agent = %agent_id_notif,
            protocol = ?init.protocol_version,
            load_session = init.agent_capabilities.load_session,
            resume_session = resume_supported,
            close_session = close_supported,
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

        // Project the per-instance MCP catalog onto ACP's typed
        // `McpServer` Vec for injection at `session/new` /
        // `session/load`. Empty when no files are configured (or all
        // entries failed projection); the agent gets an empty list and
        // runs with whatever it discovers natively.
        let mcp_servers: Vec<agent_client_protocol::schema::McpServer> = match &mcps {
            Some(reg) => reg.to_acp_servers(),
            None => Vec::new(),
        };
        // Resolved MCP count for the header `+N mcps` pill — captured
        // once at the top of the actor body and threaded through every
        // `InstanceMeta` emit. Reads via `list().len()` because the
        // registry's `count()` accessor is `cfg(test)` only.
        let mcps_count = mcps.as_ref().map(|reg| reg.list().len()).unwrap_or(0);
        if !mcp_servers.is_empty() {
            info!(
                agent = %agent_id_notif,
                count = mcp_servers.len(),
                "acp::instance: injecting mcp servers"
            );
        }

        let session_id: Option<SessionId> = match bootstrap {
            Bootstrap::Fresh => {
                debug!(agent = %agent_id_notif, "acp::instance: sending session/new");
                let mut req = NewSessionRequest::new(cwd.clone());
                req.mcp_servers = mcp_servers.clone();
                let new_session = connection.send_request(req).block_task().await?;
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
                // Captain-configured mode (`profile.mode`) wins when the
                // agent advertised it. Mirrors the model branch below.
                // Without this, the session boots in the agent's default
                // mode (`default` for claude-code) regardless of profile
                // setting — captain has to manually flip via the mode
                // picker on every spawn.
                if let Some(modes) = &new_session.modes {
                    if let Some(want) = resolved_mode.as_deref() {
                        let current = modes.current_mode_id.0.to_string();
                        let advertised = modes.available_modes.iter().any(|m| m.id.0.as_ref() == want);
                        if advertised && want != current {
                            tracing::info!(
                                agent = %agent_id_notif,
                                instance = %instance_id_notif,
                                session = %sid,
                                from = %current,
                                to = %want,
                                "acp::instance: applying configured mode via session/set_mode"
                            );
                            let req = SetSessionModeRequest::new(
                                sid.clone(),
                                SessionModeId::from(std::sync::Arc::<str>::from(want)),
                            );
                            match connection.send_request(req).block_task().await {
                                Ok(_) => {
                                    *current_mode_meta.write().await = Some(want.to_string());
                                }
                                Err(err) => {
                                    tracing::warn!(
                                        agent = %agent_id_notif,
                                        session = %sid,
                                        target_mode = %want,
                                        %err,
                                        "acp::instance: session/set_mode failed at spawn — keeping agent default"
                                    );
                                }
                            }
                        }
                    }
                }
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
                    let current = models.current_model_id.0.to_string();

                    *available_models_meta.write().await = advertised.clone();
                    *current_model_meta.write().await = Some(current.clone());

                    // Captain-configured model wins when (a) it's set, (b) the
                    // agent advertised an available_models list (so we know
                    // session/set_model is supported), and (c) it differs from
                    // the agent's default selection. Spawn-time env-var
                    // injection (claude-code's `ANTHROPIC_MODEL`,
                    // opencode's `OPENCODE_MODEL`) is best-effort — opencode
                    // in particular often resolves to its own default unless
                    // a config file backs the env, and stale parent-process
                    // env can ride through silently. set_model after
                    // session/new is the canonical lever.
                    if let Some(want) = cfg.model.as_deref() {
                        if want != current && advertised.iter().any(|m| m.id == want) {
                            tracing::info!(
                                agent = %agent_id_notif,
                                instance = %instance_id_notif,
                                session = %sid,
                                from = %current,
                                to = %want,
                                "acp::instance: applying configured model via session/set_model"
                            );
                            let req = SetSessionModelRequest::new(
                                sid.clone(),
                                ModelId::from(std::sync::Arc::<str>::from(want)),
                            );
                            match connection.send_request(req).block_task().await {
                                Ok(_) => {
                                    *current_model_meta.write().await = Some(want.to_string());
                                }
                                Err(err) => {
                                    tracing::warn!(
                                        agent = %agent_id_notif,
                                        session = %sid,
                                        target_model = %want,
                                        %err,
                                        "acp::instance: session/set_model failed at spawn — keeping agent default"
                                    );
                                }
                            }
                        }
                    }
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
                    profile_id: resolved_profile_id.clone(),
                    session_id: Some(sid.0.to_string()),
                    cwd: cwd_str.clone(),
                    current_mode_id: current_mode_meta.read().await.clone(),
                    current_model_id: current_model_meta.read().await.clone(),
                    available_modes: available_modes_meta.read().await.clone(),
                    available_models: available_models_meta.read().await.clone(),
                    mcps_count,
                });
                Some(sid)
            }
            Bootstrap::Resume(sid) => {
                let sid = SessionId::new(sid);
                // Prefer `session/load` when advertised — it's the
                // method that actually replays prior history as
                // `session/update` notifications. claude-code-acp
                // advertises `session/resume` too but resume returns
                // success without re-streaming the transcript, so
                // restored sessions render empty. Fall back to resume
                // only for vendors that ship resume but not load.
                if !resume_supported && !load_supported {
                    warn!(
                        agent = %agent_id_notif,
                        "acp::instance: neither session/resume nor session/load advertised by agent"
                    );
                    let _ = events_tx_notif.send(InstanceEvent::State {
                        agent_id: agent_id_notif.clone(),
                        instance_id: instance_id_notif.clone(),
                        session_id: Some(sid.0.to_string()),
                        state: InstanceState::Error,
                    });
                    return Err(
                        agent_client_protocol::Error::method_not_found().data(serde_json::json!({
                            "reason": format!("{}: neither session/resume nor session/load supported", agent_id_notif),
                        })),
                    );
                }
                {
                    let mut slot = session_id_forward.write().await;
                    *slot = Some(sid.clone());
                }
                // Read mode + model state off whichever response we
                // got. Resume + Load share the same `modes` / `models`
                // shape; collapse both branches via a tiny tuple to
                // avoid duplicating the projection logic.
                let (modes_state, models_state) = if load_supported {
                    debug!(agent = %agent_id_notif, session = %sid, "acp::instance: sending session/load");
                    let mut load_req = LoadSessionRequest::new(sid.clone(), cwd.clone());
                    load_req.mcp_servers = mcp_servers.clone();
                    let load_resp = match connection.send_request(load_req).block_task().await {
                        Ok(resp) => resp,
                        Err(err) => {
                            warn!(agent = %agent_id_notif, %err, "acp::instance: session/load failed");
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
                    (load_resp.modes, load_resp.models)
                } else {
                    use agent_client_protocol::schema::ResumeSessionRequest;
                    debug!(agent = %agent_id_notif, session = %sid, "acp::instance: sending session/resume");
                    let mut req = ResumeSessionRequest::new(sid.clone(), cwd.clone());
                    req.mcp_servers = mcp_servers.clone();
                    let resp = match connection.send_request(req).block_task().await {
                        Ok(resp) => resp,
                        Err(err) => {
                            warn!(agent = %agent_id_notif, %err, "acp::instance: session/resume failed");
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
                        "acp::instance: session/resume accepted"
                    );
                    (resp.modes, resp.models)
                };
                // Mirror the Fresh path's `NewSessionResponse.modes/models`
                // read against `(Resume|Load)SessionResponse`. Both
                // share the same `modes` / `models` shape — collapsing
                // here keeps the projection in one spot.
                if let Some(modes) = &modes_state {
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
                if let Some(models) = &models_state {
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
                // Replay is a snapshot, not a live turn. The dispatch
                // loop hasn't yet drained the queued session/update
                // notifications (it can't — we're inside the request
                // future), so any synthetic turn the replay will mint
                // doesn't exist YET. Spawn a deferred close that fires
                // after a short quiet window: by the time it runs the
                // dispatch loop has processed the queued events + the
                // synthetic turn id is set; we then emit TurnEnded so
                // the UI's "running" indicator clears. The close
                // checks `current_turn_id == synthetic_id` to avoid
                // racing a real prompt that arrived in the interim.
                {
                    const REPLAY_DRAIN_QUIET_MS: u64 = 1500;
                    let synth_slot = synthetic_turn_id.clone();
                    let current_slot = current_turn_id.clone();
                    let events = events_tx_notif.clone();
                    let agent_id = agent_id_notif.clone();
                    let instance_id = instance_id_notif.clone();
                    let session_id = sid.0.to_string();
                    tokio::spawn(async move {
                        tokio::time::sleep(std::time::Duration::from_millis(REPLAY_DRAIN_QUIET_MS)).await;
                        let synth = match synth_slot.write().await.take() {
                            Some(s) => s,
                            None => return,
                        };
                        let mut current_guard = current_slot.write().await;
                        if current_guard.as_deref() == Some(synth.as_str()) {
                            *current_guard = None;
                        }
                        drop(current_guard);
                        debug!(
                            agent = %agent_id,
                            session = %session_id,
                            turn = %synth,
                            "acp::instance: closing synthetic turn after session restore replay"
                        );
                        let _ = events.send(InstanceEvent::TurnEnded {
                            agent_id,
                            instance_id,
                            session_id,
                            turn_id: synth,
                            stop_reason: Some("replay_complete".into()),
                            error: None,
                        });
                    });
                }
                // Resumed sessions already saw the system prompt in their
                // original turn — re-injecting it on the first post-restore
                // submit would duplicate it in agent context. Drop the
                // pending injection on this path; the Fresh-bootstrap arm
                // keeps it for first prompts on new sessions.
                if first_message_prefix.is_some() {
                    debug!(
                        agent = %agent_id_notif,
                        session = %sid,
                        "acp::instance: dropping pending system-prompt injection (session restore)"
                    );
                    first_message_prefix = None;
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
                    profile_id: resolved_profile_id.clone(),
                    session_id: Some(sid.0.to_string()),
                    cwd: cwd_str.clone(),
                    current_mode_id: current_mode_meta.read().await.clone(),
                    current_model_id: current_model_meta.read().await.clone(),
                    available_modes: available_modes_meta.read().await.clone(),
                    available_models: available_models_meta.read().await.clone(),
                    mcps_count,
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
                            // First-prompt system-prompt injection: wrap the prompt
                            // text in an Attachment-shaped wire resource, NOT
                            // concatenated with the user's text. The transcript
                            // surfaces only what the captain actually typed; the
                            // wire ships the system prompt as a markdown resource
                            // alongside any user attachments. Cleared after the
                            // first submit consumes it (one-shot per spawn).
                            let system_prompt_attachment = first_message_prefix.take().map(|prefix| Attachment {
                                slug: "system-prompt".into(),
                                path: std::path::PathBuf::from("system-prompt.md"),
                                body: prefix,
                                title: Some("system prompt".into()),
                                data: None,
                                mime: Some("text/markdown".into()),
                            });
                            // Real prompt — if a synthetic out-of-turn turn
                            // is open, close it cleanly before starting
                            // the real one.
                            if let Some(prev) = synthetic_turn_id.write().await.take() {
                                debug!(
                                    agent = %agent_id_notif,
                                    session = %sid,
                                    turn = %prev,
                                    "acp::instance: closing synthetic turn before real prompt"
                                );
                                let _ = events_tx_notif.send(InstanceEvent::TurnEnded {
                                    agent_id: agent_id_notif.clone(),
                                    instance_id: instance_id_notif.clone(),
                                    session_id: sid.0.to_string(),
                                    turn_id: prev,
                                    stop_reason: Some("superseded".into()),
                                    error: None,
                                });
                            }
                            let turn_id = uuid::Uuid::new_v4().to_string();
                            info!(
                                agent = %agent_id_notif,
                                session = %sid,
                                turn = %turn_id,
                                text_len = text.len(),
                                attachments = attachments.len(),
                                system_prompt_injected = system_prompt_attachment.is_some(),
                                "acp::instance: turn start (session/prompt)"
                            );
                            let guard = TurnGuard::new(
                                turn_id.clone(),
                                agent_id_notif.clone(),
                                instance_id_notif.clone(),
                                sid.0.to_string(),
                                events_tx_notif.clone(),
                                current_turn_id.clone(),
                            )
                            .await;
                            // Daemon-authoritative user-prompt transcript item:
                            // emitted at submit time so the UI no longer mirrors
                            // optimistically. The system-prompt attachment, when
                            // present, is intentionally NOT included here — it's
                            // a wire-side prepend the agent sees, not something
                            // the captain typed.
                            let _ = events_tx_notif.send(InstanceEvent::Transcript {
                                agent_id: agent_id_notif.clone(),
                                instance_id: instance_id_notif.clone(),
                                session_id: sid.0.to_string(),
                                turn_id: Some(turn_id.clone()),
                                item: crate::adapters::TranscriptItem::UserPrompt {
                                    text: text.clone(),
                                    attachments: attachments.clone(),
                                },
                                // User-prompt items are minted daemon-side
                                // from the captain's submit, not from a
                                // session/update notification — no `_meta`
                                // envelope to forward.
                                meta: None,
                            });
                            // Wire blocks: [system_prompt?, ...user_attachments, user_text].
                            // Per-attachment ordering preserved through the chained iterator;
                            // `build_prompt_blocks` already lays attachments before text.
                            let wire_attachments: Vec<Attachment> = system_prompt_attachment
                                .into_iter()
                                .chain(attachments.iter().cloned())
                                .collect();
                            let blocks = build_prompt_blocks(&text, &wire_attachments);
                            let conn = connection.clone();
                            let agent_log = agent_id_notif.clone();
                            let session_log = sid.clone();
                            let events_tx_done = events_tx_notif.clone();
                            let agent_id_done = agent_id_notif.clone();
                            let instance_id_done = instance_id_notif.clone();
                            let profile_id_done = resolved_profile_id.clone();
                            let turn_id_done = turn_id.clone();
                            let cwd_done = cwd_str.clone();
                            let current_mode_done = current_mode_meta.clone();
                            let current_model_done = current_model_meta.clone();
                            let available_modes_done = available_modes_meta.clone();
                            let available_models_done = available_models_meta.clone();
                            tokio::spawn(async move {
                                // `guard` owns the open-turn slot for the lifetime of
                                // this future. On panic / drop / unwind it synthesises
                                // `TurnEnded { stop_reason: "cancelled" }` so the UI
                                // never gets stuck on a phantom in-flight turn.
                                let guard = guard;
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
                                // Guard's `complete` returns true when this future
                                // still owned the slot. False = a concurrent Cancel
                                // already synthesised TurnEnded, so we skip the
                                // InstanceMeta refresh too (it piggy-backs on the
                                // emit and only fires once per logical close).
                                if !guard.complete(stop_reason, error_msg).await {
                                    let _ = reply.send(mapped);
                                    return;
                                }
                                // Refresh tick after every turn end so the
                                // header chrome re-syncs even when the agent
                                // didn't push a `current_mode_update` /
                                // `session_info_update` notification this turn.
                                let _ = events_tx_done.send(InstanceEvent::InstanceMeta {
                                    agent_id: agent_id_done,
                                    instance_id: instance_id_done,
                                    profile_id: profile_id_done,
                                    session_id: Some(sid.0.to_string()),
                                    cwd: cwd_done,
                                    current_mode_id: current_mode_done.read().await.clone(),
                                    current_model_id: current_model_done.read().await.clone(),
                                    available_modes: available_modes_done.read().await.clone(),
                                    available_models: available_models_done.read().await.clone(),
                                    mcps_count,
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
                            // Take the open turn id BEFORE sending the notification
                            // so the prompt-future's late reply finds an empty slot
                            // and skips its own `TurnEnded` emit (see
                            // `still_owned_turn` above). Synthesize `TurnEnded
                            // (cancelled)` straight away so the chat surface stops
                            // grouping post-cancel emissions onto the cancelled
                            // block and the next user submit lands in a fresh turn.
                            let cancelled_turn_id = current_turn_id.write().await.take();
                            let res = connection
                                .send_notification(CancelNotification::new(sid.clone()))
                                .map_err(|e| e.to_string());

                            if let Some(turn_id) = cancelled_turn_id {
                                let _ = events_tx_notif.send(InstanceEvent::TurnEnded {
                                    agent_id: agent_id_notif.clone(),
                                    instance_id: instance_id_notif.clone(),
                                    session_id: sid.0.to_string(),
                                    turn_id,
                                    stop_reason: Some("cancelled".to_string()),
                                    error: None,
                                });
                                // InstanceMeta refresh — same shape as the prompt-
                                // future path, kept in sync so the header chrome
                                // doesn't lag a stale mode / model after cancel.
                                let _ = events_tx_notif.send(InstanceEvent::InstanceMeta {
                                    agent_id: agent_id_notif.clone(),
                                    instance_id: instance_id_notif.clone(),
                                    profile_id: resolved_profile_id.clone(),
                                    session_id: Some(sid.0.to_string()),
                                    cwd: cwd_str.clone(),
                                    current_mode_id: current_mode_meta.read().await.clone(),
                                    current_model_id: current_model_meta.read().await.clone(),
                                    available_modes: available_modes_meta.read().await.clone(),
                                    available_models: available_models_meta.read().await.clone(),
                                    mcps_count,
                                });
                            }
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
                            let profile_log = resolved_profile_id.clone();
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
                                        profile_id: profile_log,
                                        session_id: Some(session_log.0.to_string()),
                                        cwd: cwd_done,
                                        current_mode_id: Some(mode_id),
                                        current_model_id: current_model_done.read().await.clone(),
                                        available_modes: available_modes.read().await.clone(),
                                        available_models: available_models_done.read().await.clone(),
                                        mcps_count,
                                    });
                                }
                                let _ = reply.send(res.map(|_| ()));
                            });
                        }
                        InstanceCommand::SetConfigOption { config_id, value, reply } => {
                            let Some(sid) = session_id.clone() else {
                                let _ = reply.send(Err("no live session in list-only actor".into()));
                                continue;
                            };
                            info!(
                                agent = %agent_id_notif,
                                session = %sid,
                                config_id,
                                value,
                                "acp::instance: session/set_config_option requested"
                            );
                            let conn = connection.clone();
                            tokio::spawn(async move {
                                use agent_client_protocol::schema::{SessionConfigId, SessionConfigValueId, SetSessionConfigOptionRequest};
                                let req = SetSessionConfigOptionRequest::new(
                                    sid.clone(),
                                    SessionConfigId::from(std::sync::Arc::<str>::from(config_id.as_str())),
                                    SessionConfigValueId::from(std::sync::Arc::<str>::from(value.as_str())),
                                );
                                let res = conn.send_request(req).block_task().await.map_err(|e| e.to_string());
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
                            let profile_log = resolved_profile_id.clone();
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
                                        profile_id: profile_log,
                                        session_id: Some(session_log.0.to_string()),
                                        cwd: cwd_done,
                                        current_mode_id: current_mode_done.read().await.clone(),
                                        current_model_id: Some(model_id),
                                        available_modes: available_modes_done.read().await.clone(),
                                        available_models: available_models_done.read().await.clone(),
                                        mcps_count,
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
                                mcps_count,
                            };
                            let _ = reply.send(snap);
                        }
                        InstanceCommand::Shutdown { reply } => {
                            info!(
                                agent = %agent_id_notif,
                                instance = %instance_id_notif,
                                has_session = session_id.is_some(),
                                close_supported,
                                reason = "shutdown command received",
                                "acp::instance: shutting down instance"
                            );
                            if let Some(sid) = session_id.clone() {
                                if close_supported {
                                    // Graceful path: send `session/close`
                                    // and give the agent up to 500ms to
                                    // flush. The kill_on_drop fallback
                                    // (subprocess Drop) still fires
                                    // afterward for hard cleanup. ACP
                                    // gates this behind unstable_session_close
                                    // — fall through to the legacy cancel
                                    // path when the agent doesn't advertise
                                    // it.
                                    use agent_client_protocol::schema::CloseSessionRequest;
                                    let close_fut = connection.send_request(CloseSessionRequest::new(sid.clone())).block_task();
                                    match tokio::time::timeout(std::time::Duration::from_millis(500), close_fut).await {
                                        Ok(Ok(_)) => {
                                            debug!(
                                                agent = %agent_id_notif,
                                                session = %sid,
                                                "acp::instance: session/close acked"
                                            );
                                        }
                                        Ok(Err(err)) => {
                                            warn!(
                                                agent = %agent_id_notif,
                                                session = %sid,
                                                %err,
                                                "acp::instance: session/close failed; falling through to subprocess kill"
                                            );
                                        }
                                        Err(_elapsed) => {
                                            warn!(
                                                agent = %agent_id_notif,
                                                session = %sid,
                                                "acp::instance: session/close timed out (500ms); falling through to subprocess kill"
                                            );
                                        }
                                    }
                                } else {
                                    // Legacy path: agents without
                                    // `session_capabilities.close` only
                                    // know `cancel` to flush in-flight
                                    // turns. The kill_on_drop fallback
                                    // takes over from here.
                                    let _ = connection.send_notification(CancelNotification::new(sid));
                                }
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

                            debug!(
                                agent = %agent_id_notif,
                                session = %sid,
                                update_kind,
                                "acp::instance: session/update received"
                            );
                            let MappedSessionUpdate { mapped, meta } =
                                map_session_update(update, &mut tool_call_cache, provider_id_for_fmt.as_str());
                            // Out-of-turn detection: if a transcript-shape
                            // update arrives without an open turn, mint a
                            // synthetic id + emit TurnStarted so the chat
                            // groups the entries instead of scattering them
                            // into solo blocks. SessionInfo / CurrentMode /
                            // AvailableCommands updates DO NOT trigger a
                            // synthetic turn — they're per-session metadata.
                            let needs_synthetic_turn = matches!(mapped, MappedUpdate::Transcript(_));
                            let mut turn_id = current_turn_id.read().await.clone();
                            if needs_synthetic_turn && turn_id.is_none() {
                                let synthetic = uuid::Uuid::new_v4().to_string();
                                info!(
                                    agent = %agent_id_notif,
                                    instance = %instance_id_notif,
                                    session = %sid,
                                    turn = %synthetic,
                                    "acp::instance: synthetic turn start (out-of-turn agent activity)"
                                );
                                *current_turn_id.write().await = Some(synthetic.clone());
                                *synthetic_turn_id.write().await = Some(synthetic.clone());
                                let _ = events_tx_notif.send(InstanceEvent::TurnStarted {
                                    agent_id: agent_id_notif.clone(),
                                    instance_id: instance_id_notif.clone(),
                                    session_id: sid.clone(),
                                    turn_id: synthetic.clone(),
                                });
                                turn_id = Some(synthetic);
                            }
                            let evt: Option<InstanceEvent> = match mapped {
                                MappedUpdate::Transcript(item) => Some(InstanceEvent::Transcript {
                                    agent_id: agent_id_notif.clone(),
                                    instance_id: instance_id_notif.clone(),
                                    session_id: sid,
                                    turn_id,
                                    item,
                                    meta,
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
                            content,
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
                            let formatted = {
                                use crate::tools::formatter::registry::FormatterContext;
                                let registry = crate::adapters::acp::formatter_registry();
                                let ctx = FormatterContext {
                                    wire_name: tool.as_str(),
                                    kind: kind.as_str(),
                                    raw_input: raw_input.as_ref(),
                                    adapter: provider_id_for_fmt.as_str(),
                                    content: &content,
                                };
                                registry.dispatch(&ctx)
                            };
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
                                content,
                                options,
                                formatted,
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
                command: "/bin/false".into(),
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

    fn dummy_start_params(id: &str, events_tx: broadcast::Sender<InstanceEvent>) -> StartParams {
        StartParams {
            resolved: dummy_resolved(id),
            key: crate::adapters::InstanceKey::new_v4(),
            profile_id: None,
            events_tx,
            bootstrap: Bootstrap::Fresh,
            permissions: dummy_permissions(),
            mcps: None,
            commands_cache: None,
        }
    }

    /// Regression: starting against a child that exits immediately
    /// pushes an `Error` lifecycle event rather than hanging forever.
    /// Smoke-tests the actor shell without depending on a real agent.
    #[tokio::test(flavor = "multi_thread")]
    async fn dead_child_yields_error_state() {
        let (tx, mut rx) = broadcast::channel(8);
        let handle = AcpInstance::start(dummy_start_params("ded", tx));

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
        let handle = AcpInstance::start(dummy_start_params("ded-cancel", tx));

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
        let handle = AcpInstance::start(StartParams {
            bootstrap: Bootstrap::ListOnly,
            ..dummy_start_params("ded-list", tx)
        });

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
        let handle = AcpInstance::start(StartParams {
            bootstrap: Bootstrap::Resume("00000000-0000-0000-0000-000000000000".into()),
            ..dummy_start_params("ded-resume", tx)
        });

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
