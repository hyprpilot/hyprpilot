//! ACP `Client`-side adapter. `AcpClient` forwards every incoming
//! `fs/*` and `terminal/*` request to the typed tool layer (`tools::`)
//! and maps domain errors into `agent_client_protocol::Error`.
//! `request_permission` routes through `PermissionController`:
//! profile allowlists decide `Allow` / `Deny` without UI traffic;
//! `AskUser` emits the `acp:permission-request` event and blocks on a
//! controller-managed oneshot until the webview replies (or the
//! 10-minute waiter timeout fires).

use std::path::PathBuf;
use std::sync::Arc;

use agent_client_protocol::schema::{
    CreateTerminalRequest, CreateTerminalResponse, KillTerminalRequest, KillTerminalResponse, ReadTextFileRequest,
    ReadTextFileResponse, ReleaseTerminalRequest, ReleaseTerminalResponse, RequestPermissionOutcome,
    RequestPermissionRequest, RequestPermissionResponse, SelectedPermissionOutcome, TerminalOutputRequest,
    TerminalOutputResponse, WaitForTerminalExitRequest, WaitForTerminalExitResponse, WriteTextFileRequest,
    WriteTextFileResponse,
};
use agent_client_protocol::JsonRpcNotification;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use agent_client_protocol::schema::PermissionOptionKind;

use crate::adapters::permission::{
    pick_allow_option_id, pick_reject_option_id, Decision, DecisionContext, PermissionController, PermissionOptionView,
    PermissionOutcome, PermissionRequest, ToolCallRef, WAITER_TIMEOUT,
};
use crate::mcp::MCPsRegistry;
use crate::tools::{FsTools, Sandbox, SandboxError, TerminalToolEvent, Terminals};

use self::error::{fs_error, terminal_error};

/// Notification shape for `session/update`. Carries `update` as raw
/// JSON so the boundary doesn't depend on the upstream typed
/// `SessionUpdate` enum (which is `#[non_exhaustive]` with no
/// `#[serde(other)]` fallback). The typed projection happens at the
/// mapping step downstream — see `acp::instance::map_session_update`
/// where unknowns fall through to `TranscriptItem::Unknown`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonRpcNotification)]
#[notification(method = "session/update")]
#[serde(rename_all = "camelCase")]
pub struct SessionUpdateNotification {
    pub session_id: String,
    pub update: serde_json::Value,
}

/// Payload pushed onto the per-session forwarding channel.
#[derive(Debug, Clone)]
pub enum ClientEvent {
    /// Raw `session/update` notification — same shape as the wire
    /// type. The runtime actor maps `update` into a typed
    /// `TranscriptItem` before broadcasting upstream.
    Notification(SessionUpdateNotification),
    /// A permission request the controller bounced to the UI. The
    /// runtime actor re-broadcasts this as `InstanceEvent::PermissionRequest`.
    /// Fields mirror the final Tauri `acp:permission-request` payload
    /// so the runtime can splice in `agent_id` + `instance_id` without
    /// reshaping the rest.
    PermissionRequested {
        session_id: String,
        request_id: String,
        tool: String,
        kind: String,
        args: String,
        /// Raw `tool_call.rawInput` JSON object — passed through verbatim
        /// so UI consumers (the plan-file modal, the spec sheet) can
        /// extract structured fields like `plan` for ExitPlanMode or
        /// `command` for bash without re-parsing the collapsed `args`
        /// summary.
        raw_input: Option<serde_json::Value>,
        /// Raw `tool_call.content[]` blocks (pass-through of the ACP
        /// wire shape). Some agents (claude-code's `Switch mode`) ship
        /// the markdown body here instead of on `raw_input` — UI walks
        /// the array directly to render the right block type.
        content: Vec<serde_json::Value>,
        options: Vec<PermissionOptionView>,
    },
}

/// Project an ACP `PermissionOption` into the generic
/// `PermissionOptionView`.
pub(crate) fn option_view_from(v: &agent_client_protocol::schema::PermissionOption) -> PermissionOptionView {
    PermissionOptionView {
        option_id: v.option_id.0.to_string(),
        name: v.name.clone(),
        kind: permission_option_kind_wire(&v.kind).to_string(),
    }
}

/// Project an ACP `ToolCallUpdate` into the generic `ToolCallRef`
/// the permission flow consumes. ACP's `title` is a human-readable
/// summary ("Read package.json") OR the programmatic name when the
/// agent has no display string (MCP tools ship as
/// `mcp__server__tool`); `kind` is the closed-set category
/// ("read" / "edit" / "execute" / "other"). The downstream
/// permission decision pipeline + the UI both want the most-specific
/// identifier in `name`:
///
/// - `kind = Other` is the catch-all — for MCP tools the agent never
///   classifies them, so the wire string is "other" and the actual
///   identity is in `title` (`mcp__filesystem__read_file`). Prefer
///   the title in that case so `parse_mcp_tool_name`
///   downstream can attribute the call to its server, and the UI
///   gets a meaningful name to render.
/// - For every other kind we keep the wire string as `name` (Bash,
///   Read, …) so registered formatters key off the canonical kind.
/// - `kind_wire` stays separate (carries the original "other" /
///   "execute" / etc.) so the UI tone-mapper still drives the
///   correct theme color regardless of the name we picked.
///
/// `raw_args` pulls `command` from `raw_input` for Bash-family tools,
/// `path` for fs tools, else single-line JSON of `raw_input`.
impl From<&agent_client_protocol::schema::ToolCallUpdate> for ToolCallRef {
    fn from(update: &agent_client_protocol::schema::ToolCallUpdate) -> Self {
        let title = update.fields.title.clone();
        // ACP's `ToolKind` is `#[serde(rename_all = "snake_case")]`
        // upstream — let serde produce the wire string instead of
        // duplicating the match locally. `Other` is `#[serde(other)]`
        // so any future variant collapses onto it; the
        // `unwrap_or_else` is the safety net.
        let kind_wire = update
            .fields
            .kind
            .as_ref()
            .map(|k| serde_plain::to_string(k).unwrap_or_else(|_| "other".to_string()));
        // Prefer the agent's `title` — that's the tool's actual identity
        // (`Bash`, `Read`, `mcp__server__leaf`). Kind is a *classification*
        // (`execute`, `read`); using it as the dispatch key collapses every
        // execute-kind tool to "execute · cmd" in the formatter and breaks
        // glob-by-name in the trust store ("Bash*" never matches "execute").
        let name = title
            .clone()
            .or_else(|| kind_wire.clone())
            .unwrap_or_else(|| "tool".to_string());
        let raw_input = update.fields.raw_input.clone();
        let raw_args = raw_input.as_ref().and_then(|raw| {
            if let Some(cmd) = raw.get("command").and_then(|v| v.as_str()) {
                Some(cmd.to_string())
            } else if let Some(path) = raw.get("path").and_then(|v| v.as_str()) {
                Some(path.to_string())
            } else {
                serde_json::to_string(raw).ok()
            }
        });
        let content = update
            .fields
            .content
            .as_ref()
            .and_then(|blocks| serde_json::to_value(blocks).ok())
            .and_then(|v| match v {
                serde_json::Value::Array(a) => Some(a),
                _ => None,
            })
            .unwrap_or_default();
        ToolCallRef {
            name,
            title,
            raw_args,
            raw_input,
            kind_wire,
            content,
        }
    }
}

/// Wire string for `PermissionOptionKind` — mirrors the serde
/// `rename_all = "snake_case"` shape upstream uses. Closed match;
/// the catch-all guards against future additive variants on the
/// `#[non_exhaustive]` upstream enum (returns `"unknown"` so the
/// UI sees something rather than panicking).
fn permission_option_kind_wire(k: &PermissionOptionKind) -> &'static str {
    match k {
        PermissionOptionKind::AllowOnce => "allow_once",
        PermissionOptionKind::AllowAlways => "allow_always",
        PermissionOptionKind::RejectOnce => "reject_once",
        PermissionOptionKind::RejectAlways => "reject_always",
        _ => "unknown",
    }
}

/// Per-session adapter: bundles the event sender, the fs tools, and
/// the terminal registry the ACP runtime closures invoke. Cloned into
/// each `on_receive_*` closure.
#[derive(Clone)]
pub struct AcpClient {
    events: mpsc::UnboundedSender<ClientEvent>,
    fs: Arc<FsTools>,
    terminals: Arc<Terminals>,
    permissions: Arc<dyn PermissionController>,
    /// Owning instance UUID. Stamped onto every `PermissionRequest`
    /// the controller registers so `permissions/pending` can address
    /// the originating instance without a session-id hop. Also keys
    /// the `PermissionController`'s runtime trust store at decide
    /// time.
    instance_id: Option<String>,
    /// Per-instance MCP catalog — built at spawn time from
    /// `effective_mcp_files_for(profile)`. `None` when no MCP files
    /// were configured (or all files failed to load); the decision
    /// pipeline's per-server lane short-circuits to a miss in that
    /// case and the call falls through to AskUser. Owns the typed
    /// `hyprpilot.autoAcceptTools` / `autoRejectTools` lists used by
    /// `PermissionController::decide` lane 2.
    mcps: Option<Arc<MCPsRegistry>>,
}

impl std::fmt::Debug for AcpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AcpClient")
            .field("fs", &self.fs)
            .field("terminals", &self.terminals)
            .field("instance_id", &self.instance_id)
            .finish_non_exhaustive()
    }
}

impl AcpClient {
    pub fn with_instance_id(
        events: mpsc::UnboundedSender<ClientEvent>,
        sandbox_root: PathBuf,
        permissions: Arc<dyn PermissionController>,
        mcps: Option<Arc<MCPsRegistry>>,
        instance_id: Option<String>,
    ) -> Result<Self, SandboxError> {
        let sandbox = Sandbox::new(sandbox_root)?;
        Ok(Self {
            events,
            fs: Arc::new(FsTools::new(sandbox.clone())),
            terminals: Arc::new(Terminals::new(sandbox)),
            permissions,
            instance_id,
            mcps,
        })
    }

    /// Forward a raw session/update notification to the actor. Closed
    /// receiver degrades to a trace line — the actor has already
    /// finished its select loop.
    pub fn forward_notification(&self, notification: SessionUpdateNotification) {
        tracing::trace!(
            session = %notification.session_id,
            update_kind = ?notification.update.get("sessionUpdate").and_then(|v| v.as_str()),
            "acp::client: received session/update notification"
        );
        if self.events.send(ClientEvent::Notification(notification)).is_err() {
            tracing::trace!("acp::client: events channel closed, dropping notification");
        }
    }

    /// Handle `session/request_permission`. Runs the
    /// `PermissionController` decision chain: profile reject globs
    /// deny, profile accept globs allow, otherwise emit
    /// `acp:permission-request` and block on the controller oneshot
    /// until the UI replies or the 10-minute timeout fires.
    pub async fn request_permission(
        &self,
        req: &RequestPermissionRequest,
    ) -> Result<RequestPermissionResponse, agent_client_protocol::Error> {
        let options = req.options.iter().map(option_view_from).collect::<Vec<_>>();
        let tool_call = ToolCallRef::from(&req.tool_call);
        let request_id = uuid::Uuid::new_v4().to_string();

        let decision_req = PermissionRequest {
            instance_id: self.instance_id.clone(),
            request_id: request_id.clone(),
            tool_call: tool_call.clone(),
            options: options.clone(),
        };

        let ctx = DecisionContext {
            mcps: self.mcps.as_deref(),
        };
        match self.permissions.decide(&decision_req, &ctx) {
            Decision::Allow => {
                tracing::info!(
                    session = %req.session_id,
                    tool = %tool_call.name,
                    instance_id = ?self.instance_id,
                    "acp::client: permission auto-accepted by per-server glob"
                );
                let opt_id = pick_allow_option_id(&decision_req.options).ok_or_else(|| {
                    agent_client_protocol::Error::internal_error()
                        .data("auto-accept resolved but agent offered no options")
                })?;
                tracing::debug!(
                    session = %req.session_id,
                    tool = %tool_call.name,
                    option_id = %opt_id,
                    "acp::client: picked allow option id"
                );
                Ok(RequestPermissionResponse::new(RequestPermissionOutcome::Selected(
                    SelectedPermissionOutcome::new(agent_client_protocol::schema::PermissionOptionId::new(opt_id)),
                )))
            }
            Decision::Deny => {
                tracing::info!(
                    session = %req.session_id,
                    tool = %tool_call.name,
                    instance_id = ?self.instance_id,
                    "acp::client: permission auto-rejected by trust store / per-server glob"
                );
                match pick_reject_option_id(&decision_req.options) {
                    Some(opt_id) => {
                        tracing::debug!(
                            session = %req.session_id,
                            tool = %tool_call.name,
                            option_id = %opt_id,
                            "acp::client: picked reject option id"
                        );
                        Ok(RequestPermissionResponse::new(RequestPermissionOutcome::Selected(
                            SelectedPermissionOutcome::new(agent_client_protocol::schema::PermissionOptionId::new(
                                opt_id,
                            )),
                        )))
                    }
                    None => {
                        tracing::debug!(
                            session = %req.session_id,
                            tool = %tool_call.name,
                            "acp::client: no reject option offered, falling through to Cancelled"
                        );
                        Ok(RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled))
                    }
                }
            }
            Decision::AskUser => {
                let tool = tool_call.name.clone();
                let kind = tool_call.permission_kind_wire();
                let args = tool_call.raw_args.clone().unwrap_or_else(|| tool.clone());
                let raw_input = tool_call.raw_input.clone();
                let content = tool_call.content.clone();

                let rx = self.permissions.register_pending(decision_req.clone()).await;

                let _ = self.events.send(ClientEvent::PermissionRequested {
                    session_id: req.session_id.0.to_string(),
                    request_id: request_id.clone(),
                    tool,
                    kind,
                    args,
                    raw_input,
                    content,
                    options,
                });
                tracing::info!(
                    session = %req.session_id,
                    request_id = %request_id,
                    tool = %tool_call.name,
                    "acp::client: permission AskUser emitted, awaiting reply"
                );

                match tokio::time::timeout(WAITER_TIMEOUT, rx).await {
                    Ok(Ok(PermissionOutcome::Selected(opt_id))) => {
                        tracing::info!(
                            session = %req.session_id,
                            request_id = %request_id,
                            tool = %tool_call.name,
                            option_id = %opt_id,
                            "acp::client: permission reply resolved"
                        );
                        Ok(RequestPermissionResponse::new(RequestPermissionOutcome::Selected(
                            SelectedPermissionOutcome::new(agent_client_protocol::schema::PermissionOptionId::new(
                                opt_id,
                            )),
                        )))
                    }
                    Ok(Ok(PermissionOutcome::Cancelled)) | Ok(Err(_)) => {
                        tracing::info!(
                            session = %req.session_id,
                            request_id = %request_id,
                            tool = %tool_call.name,
                            "acp::client: permission reply resolved as Cancelled"
                        );
                        Ok(RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled))
                    }
                    Err(_elapsed) => {
                        tracing::warn!(
                            session = %req.session_id,
                            request_id = %request_id,
                            tool = %tool_call.name,
                            timeout_secs = WAITER_TIMEOUT.as_secs(),
                            "acp::client: permission waiter timed out, resolving as Cancelled"
                        );
                        self.permissions.forget(&request_id).await;
                        Ok(RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled))
                    }
                }
            }
        }
    }

    pub async fn read_text_file(
        &self,
        req: &ReadTextFileRequest,
    ) -> Result<ReadTextFileResponse, agent_client_protocol::Error> {
        tracing::debug!(
            session = %req.session_id,
            path = %req.path.display(),
            line = ?req.line,
            limit = ?req.limit,
            "acp::client: tool call fs/read_text_file"
        );
        let content = self.fs.read(&req.path, req.line, req.limit).await.map_err(fs_error)?;
        Ok(ReadTextFileResponse::new(content))
    }

    pub async fn write_text_file(
        &self,
        req: &WriteTextFileRequest,
    ) -> Result<WriteTextFileResponse, agent_client_protocol::Error> {
        tracing::debug!(
            session = %req.session_id,
            path = %req.path.display(),
            bytes = req.content.len(),
            "acp::client: tool call fs/write_text_file"
        );
        self.fs.write(&req.path, &req.content).await.map_err(fs_error)?;
        Ok(WriteTextFileResponse::new())
    }

    pub async fn create_terminal(
        &self,
        req: &CreateTerminalRequest,
    ) -> Result<CreateTerminalResponse, agent_client_protocol::Error> {
        tracing::debug!(
            session = %req.session_id,
            command = %req.command,
            args_count = req.args.len(),
            "acp::client: tool call terminal/create"
        );
        self.terminals
            .create(req.session_id.0.as_ref(), req.clone())
            .await
            .map_err(terminal_error)
    }

    pub async fn terminal_output(
        &self,
        req: &TerminalOutputRequest,
    ) -> Result<TerminalOutputResponse, agent_client_protocol::Error> {
        tracing::debug!(
            session = %req.session_id,
            terminal_id = %req.terminal_id.0,
            "acp::client: tool call terminal/output"
        );
        self.terminals
            .output(req.session_id.0.as_ref(), req.clone())
            .await
            .map_err(terminal_error)
    }

    pub async fn wait_for_terminal_exit(
        &self,
        req: &WaitForTerminalExitRequest,
    ) -> Result<WaitForTerminalExitResponse, agent_client_protocol::Error> {
        tracing::debug!(
            session = %req.session_id,
            terminal_id = %req.terminal_id.0,
            "acp::client: tool call terminal/wait_for_exit"
        );
        self.terminals
            .wait(req.session_id.0.as_ref(), req.clone())
            .await
            .map_err(terminal_error)
    }

    pub async fn kill_terminal(
        &self,
        req: &KillTerminalRequest,
    ) -> Result<KillTerminalResponse, agent_client_protocol::Error> {
        tracing::debug!(
            session = %req.session_id,
            terminal_id = %req.terminal_id.0,
            "acp::client: tool call terminal/kill"
        );
        self.terminals
            .kill(req.session_id.0.as_ref(), req.clone())
            .await
            .map_err(terminal_error)
    }

    pub async fn release_terminal(
        &self,
        req: &ReleaseTerminalRequest,
    ) -> Result<ReleaseTerminalResponse, agent_client_protocol::Error> {
        tracing::debug!(
            session = %req.session_id,
            terminal_id = %req.terminal_id.0,
            "acp::client: tool call terminal/release"
        );
        self.terminals
            .release(req.session_id.0.as_ref(), req.clone())
            .await
            .map_err(terminal_error)
    }

    /// Drain every terminal registered under `session_id`. Called from
    /// the runtime actor's tail cleanup so per-session child processes
    /// never outlive the agent connection.
    pub async fn drain_terminals_for_session(&self, session_id: &agent_client_protocol::schema::SessionId) {
        self.terminals.drain_for(session_id.0.as_ref()).await;
    }

    /// Subscribe to live stdout / stderr / exit chunks for every
    /// terminal this client owns. The runtime actor subscribes once at
    /// startup and re-publishes each chunk as an
    /// `InstanceEvent::Terminal` stamped with `(agent_id, instance_id,
    /// session_id, turn_id?)`. Consumers must handle
    /// `broadcast::error::RecvError::Lagged` — the channel silently
    /// drops messages otherwise.
    #[must_use]
    pub fn subscribe_terminals(&self) -> tokio::sync::broadcast::Receiver<TerminalToolEvent> {
        self.terminals.subscribe()
    }
}

mod error {
    use crate::tools::{FsError, SandboxError, TerminalError};

    pub(super) fn fs_error(err: FsError) -> agent_client_protocol::Error {
        match err {
            FsError::Sandbox(inner) => sandbox_error(inner),
            FsError::Io { .. } => agent_client_protocol::Error::internal_error().data(err.to_string()),
        }
    }

    pub(super) fn terminal_error(err: TerminalError) -> agent_client_protocol::Error {
        match err {
            TerminalError::Sandbox(inner) => sandbox_error(inner),
            TerminalError::UnknownTerminal(_) => agent_client_protocol::Error::invalid_params().data(err.to_string()),
            TerminalError::ExitStatusUnavailable | TerminalError::Io(_) => {
                agent_client_protocol::Error::internal_error().data(err.to_string())
            }
        }
    }

    fn sandbox_error(err: SandboxError) -> agent_client_protocol::Error {
        agent_client_protocol::Error::invalid_params().data(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::permission::DefaultPermissionController;
    use agent_client_protocol::schema::{
        PermissionOption, PermissionOptionId, PermissionOptionKind, RequestPermissionRequest, SessionId, ToolCallId,
        ToolCallUpdate, ToolCallUpdateFields, ToolKind,
    };
    use std::path::PathBuf;

    fn mk_client(dir: &std::path::Path) -> (AcpClient, mpsc::UnboundedReceiver<ClientEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let controller = Arc::new(DefaultPermissionController::new()) as Arc<dyn PermissionController>;
        (
            AcpClient::with_instance_id(tx, dir.to_path_buf(), controller, None, None).expect("sandbox constructs"),
            rx,
        )
    }

    fn mk_client_with_mcps(
        dir: &std::path::Path,
        mcps: Arc<MCPsRegistry>,
    ) -> (AcpClient, mpsc::UnboundedReceiver<ClientEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let controller = Arc::new(DefaultPermissionController::new()) as Arc<dyn PermissionController>;
        (
            AcpClient::with_instance_id(
                tx,
                dir.to_path_buf(),
                controller,
                Some(mcps),
                Some("instance-test".into()),
            )
            .expect("sandbox constructs"),
            rx,
        )
    }

    fn sample_permission_request(kind: ToolKind) -> RequestPermissionRequest {
        let fields = ToolCallUpdateFields::new().kind(kind).title("Bash");
        RequestPermissionRequest::new(
            SessionId::new("sess-1"),
            ToolCallUpdate::new(ToolCallId::new("tc-1"), fields),
            vec![
                PermissionOption::new(
                    PermissionOptionId::new("allow-once"),
                    "Allow",
                    PermissionOptionKind::AllowOnce,
                ),
                PermissionOption::new(
                    PermissionOptionId::new("reject-once"),
                    "Reject",
                    PermissionOptionKind::RejectOnce,
                ),
            ],
        )
    }

    #[tokio::test]
    async fn permission_request_without_profile_awaits_ui() {
        let dir = tempfile::tempdir().unwrap();
        let (client, mut rx) = mk_client(dir.path());
        let req = sample_permission_request(ToolKind::Execute);

        // Kick off request_permission concurrently, then resolve via
        // the event the client emits + a manual controller resolve.
        // Without a profile, decide() returns AskUser → event fires.
        let client_clone = client.clone();
        let handle = tokio::spawn(async move { client_clone.request_permission(&req).await });

        // Drain the event and short-circuit the controller by
        // resolving directly — since the test uses the default
        // controller we grab it via the Arc clone.
        let evt = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("event emitted")
            .expect("channel open");
        let (request_id, tool, kind, args) = match evt {
            ClientEvent::PermissionRequested {
                request_id,
                tool,
                kind,
                args,
                ..
            } => (request_id, tool, kind, args),
            other => panic!("expected PermissionRequested, got {other:?}"),
        };
        assert!(!request_id.is_empty());
        // `tool` is the agent's title (e.g. "Bash"); kind is the
        // ACP-spec classification (`execute`).
        assert_eq!(tool, "Bash");
        assert_eq!(kind, "execute");
        // `args` falls back to the tool name when raw_args is unset.
        assert_eq!(args, "Bash");

        // Drop the handle; rx close resolves request_permission to Cancelled.
        client
            .permissions
            .resolve(&request_id, PermissionOutcome::Cancelled)
            .await;
        let resp = handle.await.expect("join").expect("ok");
        assert!(matches!(resp.outcome, RequestPermissionOutcome::Cancelled));
    }

    #[tokio::test]
    async fn permission_request_with_per_server_accept_short_circuits_ui() {
        // Per-server hyprpilot.autoAcceptTools entry matches the tool;
        // decide() returns Allow without prompting the UI. Title is
        // the discriminator here — `From<&ToolCallUpdate>` falls back
        // to the title when no `kind` is set, so the
        // `mcp__filesystem__read_file` name reaches `decide()` for
        // prefix-parsing.
        use crate::mcp::{HyprpilotExtension, MCPDefinition};
        use serde_json::json;

        let dir = tempfile::tempdir().unwrap();
        let mcps = Arc::new(MCPsRegistry::new(vec![MCPDefinition {
            name: "filesystem".into(),
            raw: json!({ "command": "echo" }),
            hyprpilot: HyprpilotExtension {
                auto_accept_tools: vec!["read_*".into()],
                auto_reject_tools: vec![],
            },
            source: PathBuf::from("test.json"),
        }]));
        let (client, mut rx) = mk_client_with_mcps(dir.path(), mcps);

        let fields = ToolCallUpdateFields::new().title("mcp__filesystem__read_file");
        let req = RequestPermissionRequest::new(
            SessionId::new("sess-1"),
            ToolCallUpdate::new(ToolCallId::new("tc-1"), fields),
            vec![PermissionOption::new(
                PermissionOptionId::new("allow-once"),
                "Allow",
                PermissionOptionKind::AllowOnce,
            )],
        );

        let resp = client.request_permission(&req).await.expect("auto-accept");
        match resp.outcome {
            RequestPermissionOutcome::Selected(sel) => {
                assert_eq!(&*sel.option_id.0, "allow-once");
            }
            other => panic!("expected Selected, got {other:?}"),
        }
        assert!(rx.try_recv().is_err(), "no UI event for per-server accept decisions");
    }

    #[test]
    fn tool_call_ref_other_kind_falls_back_to_title() {
        // ACP `kind = Other` is the catch-all for MCP / unmapped
        // tools. The downstream permission decision pipeline + the
        // UI both want the most-specific identifier — for MCP tools
        // that's the programmatic name in `title`
        // (`mcp__filesystem__read_file`), not the literal "other"
        // wire string. Pin the From conversion so the regression
        // path stays closed.
        let fields = ToolCallUpdateFields::new()
            .kind(ToolKind::Other)
            .title("mcp__filesystem__read_file");
        let update = ToolCallUpdate::new(ToolCallId::new("tc-1"), fields);
        let tool_ref = ToolCallRef::from(&update);
        assert_eq!(tool_ref.name, "mcp__filesystem__read_file");
        assert_eq!(tool_ref.kind_wire.as_deref(), Some("other"));
        assert_eq!(tool_ref.title.as_deref(), Some("mcp__filesystem__read_file"));
    }

    #[test]
    fn tool_call_ref_prefers_title_over_kind_wire() {
        // The agent's `title` is the tool's actual identity (`Bash`,
        // `Read`, `mcp__server__leaf`); kind is just a classification
        // (`execute`, `read`). Formatter dispatch + trust-store globs
        // both key on `name`, so we want the identity, not the verb.
        let fields = ToolCallUpdateFields::new().kind(ToolKind::Execute).title("Bash");
        let update = ToolCallUpdate::new(ToolCallId::new("tc-1"), fields);
        let tool_ref = ToolCallRef::from(&update);
        assert_eq!(tool_ref.name, "Bash");
        assert_eq!(tool_ref.kind_wire.as_deref(), Some("execute"));
        assert_eq!(tool_ref.title.as_deref(), Some("Bash"));
    }

    #[test]
    fn tool_call_ref_falls_back_to_kind_wire_without_title() {
        // No title → fall back to the kind classification so glob
        // matching still has something to work with.
        let fields = ToolCallUpdateFields::new().kind(ToolKind::Execute);
        let update = ToolCallUpdate::new(ToolCallId::new("tc-1"), fields);
        let tool_ref = ToolCallRef::from(&update);
        assert_eq!(tool_ref.name, "execute");
    }

    #[test]
    fn permission_option_view_maps_all_kinds() {
        for (kind, wire) in [
            (PermissionOptionKind::AllowOnce, "allow_once"),
            (PermissionOptionKind::AllowAlways, "allow_always"),
            (PermissionOptionKind::RejectOnce, "reject_once"),
            (PermissionOptionKind::RejectAlways, "reject_always"),
        ] {
            let opt = PermissionOption::new(PermissionOptionId::new("x"), "X", kind);
            let view = option_view_from(&opt);
            assert_eq!(view.kind, wire);
        }
    }

    /// Any JSON payload — including unknown `sessionUpdate` variants —
    /// must round-trip through the tolerant notification without
    /// `SessionUpdateNotification` deserializes any variant payload,
    /// including ones the upstream typed `SessionUpdate` enum doesn't
    /// know about — the boundary doesn't depend on `SessionUpdate`'s
    /// match arms; the typed projection happens downstream.
    #[test]
    fn session_update_notification_accepts_unknown_variant() {
        let raw = serde_json::json!({
            "sessionId": "sess-1",
            "update": {
                "sessionUpdate": "some_future_variant_the_crate_does_not_know",
                "anything": { "nested": true }
            }
        });
        let n: SessionUpdateNotification = serde_json::from_value(raw.clone()).expect("raw parse");
        assert_eq!(n.session_id, "sess-1");
        assert_eq!(n.update, raw["update"]);
    }

    #[tokio::test]
    async fn read_text_file_happy_envelope() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("hello.txt"), "one\n").unwrap();

        let (client, _rx) = mk_client(dir.path());
        let req = ReadTextFileRequest::new(SessionId::new("s"), PathBuf::from("hello.txt"));
        let resp = client.read_text_file(&req).await.expect("read ok");
        assert!(resp.content.contains("one"));
    }

    #[tokio::test]
    async fn read_text_file_outside_sandbox_maps_to_invalid_params() {
        let dir = tempfile::tempdir().unwrap();
        let (client, _rx) = mk_client(dir.path());
        let req = ReadTextFileRequest::new(SessionId::new("s"), PathBuf::from("/etc/passwd"));
        let err = client.read_text_file(&req).await.expect_err("must reject");
        assert_eq!(err.code, agent_client_protocol::schema::ErrorCode::InvalidParams);
    }

    #[tokio::test]
    async fn write_text_file_outside_sandbox_maps_to_invalid_params() {
        let dir = tempfile::tempdir().unwrap();
        let (client, _rx) = mk_client(dir.path());
        let req = WriteTextFileRequest::new(SessionId::new("s"), PathBuf::from("/tmp/escape.txt"), "x".to_string());
        let err = client.write_text_file(&req).await.expect_err("must reject");
        assert_eq!(err.code, agent_client_protocol::schema::ErrorCode::InvalidParams);
    }

    #[tokio::test]
    async fn terminal_unknown_id_maps_to_invalid_params() {
        let dir = tempfile::tempdir().unwrap();
        let (client, _rx) = mk_client(dir.path());
        let req = TerminalOutputRequest::new(
            SessionId::new("s"),
            agent_client_protocol::schema::TerminalId::new("nope"),
        );
        let err = client.terminal_output(&req).await.expect_err("must fail");
        assert_eq!(err.code, agent_client_protocol::schema::ErrorCode::InvalidParams);
    }
}
