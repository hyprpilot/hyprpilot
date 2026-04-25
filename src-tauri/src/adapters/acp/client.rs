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

use crate::adapters::permission::{
    pick_allow_option_id, pick_reject_option_id, Decision, PermissionController, PermissionOptionView,
    PermissionOutcome, PermissionRequest, ToolCallRef, WAITER_TIMEOUT,
};
use crate::config::ProfileConfig;
use crate::tools::{FsTools, Sandbox, SandboxError, TerminalToolEvent, Terminals};

use self::error::{fs_error, terminal_error};

/// Tolerant mirror of `schema::SessionNotification`. Carries `update` as
/// raw JSON so we never fail `parse_message` on variants the
/// `agent-client-protocol-schema` crate version we pin doesn't know
/// about yet. The typed path fails closed — `send_error_notification`
/// then emits a `{jsonrpc, error}` envelope with no `id` that the
/// agent SDK logs as "Invalid message" (see `typed.rs:889`). Keeping
/// the payload untyped at receive time avoids the whole cascade; the
/// UI discriminates on `sessionUpdate` variants and drops unknowns.
#[derive(Debug, Clone, Serialize, Deserialize, JsonRpcNotification)]
#[notification(method = "session/update")]
#[serde(rename_all = "camelCase")]
pub struct TolerantSessionNotification {
    pub session_id: String,
    pub update: serde_json::Value,
    #[serde(rename = "_meta", default, skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Value>,
}

/// Payload pushed onto the per-session forwarding channel.
#[derive(Debug, Clone)]
pub enum ClientEvent {
    Notification {
        session_id: String,
        update: serde_json::Value,
    },
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
        options: Vec<PermissionOptionView>,
    },
}

/// Project an ACP `PermissionOption` into the generic
/// `PermissionOptionView`. Lives here (not in `mapping.rs`) because
/// the `From` impl reads the ACP `PermissionOptionKind` via
/// `wire_name` which is an ACP-local helper.
pub(crate) fn option_view_from(v: &agent_client_protocol::schema::PermissionOption) -> PermissionOptionView {
    let kind = wire_name(&v.kind).unwrap_or_else(|| {
        tracing::error!(
            kind = ?v.kind,
            "acp::client: PermissionOptionKind serialised to non-string — upstream crate shape changed"
        );
        "unknown".to_string()
    });
    PermissionOptionView {
        option_id: v.option_id.0.to_string(),
        name: v.name.clone(),
        kind,
    }
}

/// Resolved tool identity for the **permission path only** — globs-
/// match name + minimal fields for the two-button prompt. The
/// transcript-side tool chips / rows get their own vendor-aware
/// formatter in `ui/src/lib/tool-formatters.ts`.
///
/// `name` is the canonical key globs match against; `title` is ACP's
/// opaque human-readable summary; `kind_wire` is `Some` only when
/// `name` came from the closed-set `ToolKind`, so the UI can key its
/// theme off a known-short string.
struct ToolIdentity {
    name: String,
    title: Option<String>,
    kind_wire: Option<String>,
}

/// ACP `title` is a human-readable summary ("Read package.json"),
/// while `kind` is the closed-set category ("read" / "edit" /
/// "execute"). For a `[[profiles]].auto_accept_tools = ["Read", "Edit*"]`
/// style allowlist the *canonical* name is the `kind` wire string —
/// ACP doesn't currently surface a third "programmatic name" field.
/// Vendor-specific programmatic names ride in `_meta.*.toolName` but
/// that extractor is a separate issue; we match on `kind` first,
/// then the title, then a neutral `"tool"` sentinel.
fn extract_tool_identity(update: &agent_client_protocol::schema::ToolCallUpdate) -> ToolIdentity {
    let title = update.fields.title.clone();
    if let Some(k) = &update.fields.kind {
        if let Some(wire) = wire_name(k) {
            return ToolIdentity {
                name: wire.clone(),
                title,
                kind_wire: Some(wire),
            };
        }
    }
    if let Some(t) = &title {
        return ToolIdentity {
            name: t.clone(),
            title,
            kind_wire: None,
        };
    }
    ToolIdentity {
        name: "tool".to_string(),
        title,
        kind_wire: None,
    }
}

/// Short UI string summarising the tool args. Pulls `command` from
/// `raw_input` for Bash-family tools, `path` for fs tools, else
/// serialises a single-line JSON of `raw_input` when present.
fn extract_raw_args(update: &agent_client_protocol::schema::ToolCallUpdate) -> Option<String> {
    let raw = update.fields.raw_input.as_ref()?;
    if let Some(cmd) = raw.get("command").and_then(|v| v.as_str()) {
        return Some(cmd.to_string());
    }
    if let Some(path) = raw.get("path").and_then(|v| v.as_str()) {
        return Some(path.to_string());
    }
    serde_json::to_string(raw).ok()
}

/// The `kind` string the UI uses to colour the permission prompt.
/// Only emits the wire name when `extract_tool_identity` resolved it
/// from ACP's closed ToolKind enum; anything else (title fallback,
/// neutral sentinel) maps to `"acp"` so free-form English never
/// bleeds into the UI's closed-set theme map.
fn permission_kind_wire(kind_wire: Option<&str>) -> String {
    kind_wire
        .map(str::to_ascii_lowercase)
        .unwrap_or_else(|| "acp".to_string())
}

/// Map an SDK enum to its canonical wire string via its own serde
/// derive. Returns `None` when the upstream shape departs from a bare
/// JSON string (e.g. a future additive variant carrying payload) —
/// callers choose whether to log and fall back to a sentinel.
fn wire_name<T: Serialize>(value: &T) -> Option<String> {
    match serde_json::to_value(value).ok()? {
        serde_json::Value::String(s) => Some(s),
        _ => None,
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
    /// Profile bound to this instance at spawn time. `None` when the
    /// actor was started without a profile overlay (bare-agent
    /// resolution); the decision chain treats that as "ask user"
    /// unconditionally.
    profile: Option<ProfileConfig>,
    /// Owning instance UUID. Stamped onto every `PermissionRequest`
    /// the controller registers so `permissions/pending` can address
    /// the originating instance without a session-id hop.
    instance_id: Option<String>,
}

impl std::fmt::Debug for AcpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AcpClient")
            .field("fs", &self.fs)
            .field("terminals", &self.terminals)
            .field("profile", &self.profile)
            .finish_non_exhaustive()
    }
}

impl AcpClient {
    pub fn new(
        events: mpsc::UnboundedSender<ClientEvent>,
        sandbox_root: PathBuf,
        permissions: Arc<dyn PermissionController>,
        profile: Option<ProfileConfig>,
    ) -> Result<Self, SandboxError> {
        Self::with_instance_id(events, sandbox_root, permissions, profile, None)
    }

    pub fn with_instance_id(
        events: mpsc::UnboundedSender<ClientEvent>,
        sandbox_root: PathBuf,
        permissions: Arc<dyn PermissionController>,
        profile: Option<ProfileConfig>,
        instance_id: Option<String>,
    ) -> Result<Self, SandboxError> {
        let sandbox = Sandbox::new(sandbox_root)?;
        Ok(Self {
            events,
            fs: Arc::new(FsTools::new(sandbox.clone())),
            terminals: Arc::new(Terminals::new(sandbox)),
            permissions,
            profile,
            instance_id,
        })
    }

    /// Forward a raw session/update notification to the actor. Closed
    /// receiver degrades to a trace line — the actor has already
    /// finished its select loop.
    pub fn forward_notification(&self, notification: TolerantSessionNotification) {
        let TolerantSessionNotification { session_id, update, .. } = notification;
        tracing::trace!(
            session = %session_id,
            update_kind = ?update.get("sessionUpdate").and_then(|v| v.as_str()),
            "acp::client: received session/update notification"
        );
        if self
            .events
            .send(ClientEvent::Notification { session_id, update })
            .is_err()
        {
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
        let ToolIdentity {
            name: tool_name,
            title,
            kind_wire,
        } = extract_tool_identity(&req.tool_call);
        let raw_args = extract_raw_args(&req.tool_call);
        let request_id = uuid::Uuid::new_v4().to_string();

        let decision_req = PermissionRequest {
            session_id: req.session_id.0.to_string(),
            instance_id: self.instance_id.clone(),
            request_id: request_id.clone(),
            tool_call: ToolCallRef {
                name: tool_name.clone(),
                title: title.clone(),
                raw_args: raw_args.clone(),
            },
            options: options.clone(),
        };

        match self.permissions.decide(&decision_req, self.profile.as_ref()) {
            Decision::Allow => {
                tracing::info!(
                    session = %req.session_id,
                    tool = %tool_name,
                    profile = ?self.profile.as_ref().map(|p| &p.id),
                    "acp::client: permission auto-accepted by profile glob"
                );
                let opt_id = pick_allow_option_id(&decision_req.options).ok_or_else(|| {
                    agent_client_protocol::Error::internal_error()
                        .data("profile auto-accept but agent offered no options")
                })?;
                tracing::debug!(
                    session = %req.session_id,
                    tool = %tool_name,
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
                    tool = %tool_name,
                    profile = ?self.profile.as_ref().map(|p| &p.id),
                    "acp::client: permission auto-rejected by profile glob"
                );
                match pick_reject_option_id(&decision_req.options) {
                    Some(opt_id) => {
                        tracing::debug!(
                            session = %req.session_id,
                            tool = %tool_name,
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
                            tool = %tool_name,
                            "acp::client: no reject option offered, falling through to Cancelled"
                        );
                        Ok(RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled))
                    }
                }
            }
            Decision::AskUser => {
                let tool = tool_name.clone();
                let kind = permission_kind_wire(kind_wire.as_deref());
                let args = raw_args.clone().unwrap_or_else(|| tool.clone());

                let rx = self.permissions.register_pending(decision_req.clone()).await;

                let _ = self.events.send(ClientEvent::PermissionRequested {
                    session_id: req.session_id.0.to_string(),
                    request_id: request_id.clone(),
                    tool,
                    kind,
                    args,
                    options,
                });
                tracing::info!(
                    session = %req.session_id,
                    request_id = %request_id,
                    tool = %tool_name,
                    "acp::client: permission AskUser emitted, awaiting reply"
                );

                match tokio::time::timeout(WAITER_TIMEOUT, rx).await {
                    Ok(Ok(PermissionOutcome::Selected(opt_id))) => {
                        tracing::info!(
                            session = %req.session_id,
                            request_id = %request_id,
                            tool = %tool_name,
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
                            tool = %tool_name,
                            "acp::client: permission reply resolved as Cancelled"
                        );
                        Ok(RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled))
                    }
                    Err(_elapsed) => {
                        tracing::warn!(
                            session = %req.session_id,
                            request_id = %request_id,
                            tool = %tool_name,
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
            AcpClient::new(tx, dir.to_path_buf(), controller, None).expect("sandbox constructs"),
            rx,
        )
    }

    fn mk_client_with_profile(
        dir: &std::path::Path,
        profile: ProfileConfig,
    ) -> (AcpClient, mpsc::UnboundedReceiver<ClientEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let controller = Arc::new(DefaultPermissionController::new()) as Arc<dyn PermissionController>;
        (
            AcpClient::new(tx, dir.to_path_buf(), controller, Some(profile)).expect("sandbox constructs"),
            rx,
        )
    }

    fn profile_with_globs(id: &str, accept: Vec<String>, reject: Vec<String>) -> ProfileConfig {
        ProfileConfig {
            id: id.into(),
            agent: "claude-code".into(),
            model: None,
            system_prompt: None,
            system_prompt_file: None,
            auto_accept_tools: accept,
            auto_reject_tools: reject,
            mcps: None,
            skills: None,
            mode: None,
            cwd: None,
            env: Default::default(),
        }
    }

    fn sample_permission_request(kind: ToolKind) -> RequestPermissionRequest {
        let fields = ToolCallUpdateFields::new().kind(kind).title("sample tool call");
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
        assert_eq!(tool, "execute");
        assert_eq!(kind, "execute");
        assert_eq!(args, "execute");

        // Drop the handle; rx close resolves request_permission to Cancelled.
        client
            .permissions
            .resolve(&request_id, PermissionOutcome::Cancelled)
            .await;
        let resp = handle.await.expect("join").expect("ok");
        assert!(matches!(resp.outcome, RequestPermissionOutcome::Cancelled));
    }

    #[tokio::test]
    async fn permission_request_profile_auto_accept_skips_ui() {
        let dir = tempfile::tempdir().unwrap();
        let profile = profile_with_globs("p", vec!["execute".into()], vec![]);
        let (client, mut rx) = mk_client_with_profile(dir.path(), profile);
        let req = sample_permission_request(ToolKind::Execute);

        let resp = client.request_permission(&req).await.expect("accepted");
        match resp.outcome {
            RequestPermissionOutcome::Selected(sel) => {
                assert_eq!(&*sel.option_id.0, "allow-once");
            }
            other => panic!("expected Selected, got {other:?}"),
        }
        assert!(rx.try_recv().is_err(), "no UI event for profile-accept decisions");
    }

    #[tokio::test]
    async fn permission_request_profile_auto_reject_maps_to_reject_option() {
        let dir = tempfile::tempdir().unwrap();
        let profile = profile_with_globs("p", vec![], vec!["execute".into()]);
        let (client, mut rx) = mk_client_with_profile(dir.path(), profile);
        let req = sample_permission_request(ToolKind::Execute);

        let resp = client.request_permission(&req).await.expect("rejected");
        match resp.outcome {
            RequestPermissionOutcome::Selected(sel) => {
                assert_eq!(&*sel.option_id.0, "reject-once");
            }
            other => panic!("expected Selected(reject-once), got {other:?}"),
        }
        assert!(rx.try_recv().is_err(), "no UI event for profile-reject decisions");
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
    /// triggering `send_error_notification`. Pins the fix for the
    /// "Invalid message" cascade.
    #[test]
    fn tolerant_notification_accepts_unknown_variant() {
        let raw = serde_json::json!({
            "sessionId": "sess-1",
            "update": {
                "sessionUpdate": "some_future_variant_the_crate_does_not_know",
                "anything": { "nested": true }
            }
        });
        let n: TolerantSessionNotification = serde_json::from_value(raw.clone()).expect("tolerant parse");
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
