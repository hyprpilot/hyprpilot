//! ACP `Client`-side adapter. `AcpClient` forwards every incoming
//! `fs/*` and `terminal/*` request to the typed tool layer (`tools::`)
//! and maps domain errors into `agent_client_protocol::Error`.
//! Future `PermissionController` (K-6) lands as a field here so the
//! auto-`Cancelled` policy flips to a real trust-store without
//! rippling through runtime + session call sites.

use std::path::PathBuf;
use std::sync::Arc;

use agent_client_protocol::JsonRpcNotification;
use agent_client_protocol::schema::{
    CreateTerminalRequest, CreateTerminalResponse, KillTerminalRequest, KillTerminalResponse, ReadTextFileRequest,
    ReadTextFileResponse, ReleaseTerminalRequest, ReleaseTerminalResponse, RequestPermissionOutcome,
    RequestPermissionRequest, RequestPermissionResponse, TerminalOutputRequest, TerminalOutputResponse,
    WaitForTerminalExitRequest, WaitForTerminalExitResponse, WriteTextFileRequest, WriteTextFileResponse,
};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::tools::{FsTools, Sandbox, SandboxError, Terminals};

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
    PermissionRequested {
        session_id: String,
        options: Vec<PermissionOptionView>,
    },
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct PermissionOptionView {
    pub option_id: String,
    pub name: String,
    pub kind: String,
}

impl From<&agent_client_protocol::schema::PermissionOption> for PermissionOptionView {
    fn from(v: &agent_client_protocol::schema::PermissionOption) -> Self {
        Self {
            option_id: v.option_id.0.to_string(),
            name: v.name.clone(),
            kind: permission_option_kind_wire(&v.kind),
        }
    }
}

/// Serialise `PermissionOptionKind` via the crate's own serde derive so the
/// wire name always matches the SDK, even across `#[non_exhaustive]`
/// additions.
fn permission_option_kind_wire(kind: &agent_client_protocol::schema::PermissionOptionKind) -> String {
    match serde_json::to_value(kind) {
        Ok(serde_json::Value::String(s)) => s,
        other => {
            tracing::error!(
                ?other,
                ?kind,
                "acp::client: PermissionOptionKind serialised to non-string — upstream crate shape changed"
            );
            "unknown".to_string()
        }
    }
}

/// Per-session adapter: bundles the event sender, the fs tools, and
/// the terminal registry the ACP runtime closures invoke. Cloned into
/// each `on_receive_*` closure.
#[derive(Debug, Clone)]
pub struct AcpClient {
    events: mpsc::UnboundedSender<ClientEvent>,
    fs: Arc<FsTools>,
    terminals: Arc<Terminals>,
}

impl AcpClient {
    pub fn new(events: mpsc::UnboundedSender<ClientEvent>, sandbox_root: PathBuf) -> Result<Self, SandboxError> {
        let sandbox = Sandbox::new(sandbox_root)?;
        Ok(Self {
            events,
            fs: Arc::new(FsTools::new(sandbox.clone())),
            terminals: Arc::new(Terminals::new(sandbox)),
        })
    }

    /// Forward a raw session/update notification to the actor. Closed
    /// receiver degrades to a trace line — the actor has already
    /// finished its select loop.
    pub fn forward_notification(&self, notification: TolerantSessionNotification) {
        let TolerantSessionNotification {
            session_id, update, ..
        } = notification;
        if self
            .events
            .send(ClientEvent::Notification { session_id, update })
            .is_err()
        {
            tracing::trace!("acp::client: events channel closed, dropping notification");
        }
    }

    /// Handle `session/request_permission`. Emits an observability
    /// event for the webview, then auto-`Cancelled`.
    /// `PermissionController` (K-6) replaces the auto-cancel with a
    /// trust-store consult. `async` signature aligns with every other
    /// handler so `register_client_handler!` takes one shape.
    pub async fn request_permission(
        &self,
        req: &RequestPermissionRequest,
    ) -> Result<RequestPermissionResponse, agent_client_protocol::Error> {
        let options = req.options.iter().map(PermissionOptionView::from).collect::<Vec<_>>();
        let _ = self.events.send(ClientEvent::PermissionRequested {
            session_id: req.session_id.0.to_string(),
            options,
        });
        tracing::debug!(
            session = %req.session_id,
            options = req.options.len(),
            "acp::client: auto-cancelling permission request (pre-PermissionController)"
        );
        Ok(RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled))
    }

    pub async fn read_text_file(
        &self,
        req: &ReadTextFileRequest,
    ) -> Result<ReadTextFileResponse, agent_client_protocol::Error> {
        tracing::info!(
            session = %req.session_id,
            path = %req.path.display(),
            line = ?req.line,
            limit = ?req.limit,
            "acp::client: fs/read_text_file"
        );
        let content = self.fs.read(&req.path, req.line, req.limit).await.map_err(fs_error)?;
        Ok(ReadTextFileResponse::new(content))
    }

    pub async fn write_text_file(
        &self,
        req: &WriteTextFileRequest,
    ) -> Result<WriteTextFileResponse, agent_client_protocol::Error> {
        tracing::info!(
            session = %req.session_id,
            path = %req.path.display(),
            bytes = req.content.len(),
            "acp::client: fs/write_text_file"
        );
        self.fs.write(&req.path, &req.content).await.map_err(fs_error)?;
        Ok(WriteTextFileResponse::new())
    }

    pub async fn create_terminal(
        &self,
        req: &CreateTerminalRequest,
    ) -> Result<CreateTerminalResponse, agent_client_protocol::Error> {
        self.terminals
            .create(req.session_id.0.as_ref(), req.clone())
            .await
            .map_err(terminal_error)
    }

    pub async fn terminal_output(
        &self,
        req: &TerminalOutputRequest,
    ) -> Result<TerminalOutputResponse, agent_client_protocol::Error> {
        self.terminals
            .output(req.session_id.0.as_ref(), req.clone())
            .await
            .map_err(terminal_error)
    }

    pub async fn wait_for_terminal_exit(
        &self,
        req: &WaitForTerminalExitRequest,
    ) -> Result<WaitForTerminalExitResponse, agent_client_protocol::Error> {
        self.terminals
            .wait(req.session_id.0.as_ref(), req.clone())
            .await
            .map_err(terminal_error)
    }

    pub async fn kill_terminal(
        &self,
        req: &KillTerminalRequest,
    ) -> Result<KillTerminalResponse, agent_client_protocol::Error> {
        self.terminals
            .kill(req.session_id.0.as_ref(), req.clone())
            .await
            .map_err(terminal_error)
    }

    pub async fn release_terminal(
        &self,
        req: &ReleaseTerminalRequest,
    ) -> Result<ReleaseTerminalResponse, agent_client_protocol::Error> {
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
    use agent_client_protocol::schema::{
        PermissionOption, PermissionOptionId, PermissionOptionKind, RequestPermissionRequest, SessionId, ToolCallId,
        ToolCallUpdate,
    };
    use std::path::PathBuf;

    fn mk_client(dir: &std::path::Path) -> (AcpClient, mpsc::UnboundedReceiver<ClientEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (AcpClient::new(tx, dir.to_path_buf()).expect("sandbox constructs"), rx)
    }

    #[tokio::test]
    async fn permission_request_auto_denies() {
        let dir = tempfile::tempdir().unwrap();
        let (client, mut rx) = mk_client(dir.path());
        let req = RequestPermissionRequest::new(
            SessionId::new("sess-1"),
            ToolCallUpdate::new(ToolCallId::new("tc-1"), Default::default()),
            vec![PermissionOption::new(
                PermissionOptionId::new("allow"),
                "Allow",
                PermissionOptionKind::AllowOnce,
            )],
        );
        let resp = client.request_permission(&req).await.expect("permission reply ok");
        assert!(matches!(resp.outcome, RequestPermissionOutcome::Cancelled));
        let evt = rx.try_recv().expect("observability event emitted");
        assert!(matches!(evt, ClientEvent::PermissionRequested { .. }));
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
            let view: PermissionOptionView = (&opt).into();
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
