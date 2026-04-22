//! ACP `Client`-side handler. `AcpClient` is the composition root for
//! the per-session adapter: it owns the event channel and carries the
//! tools the runtime actor plugs into `Client.builder().on_receive_*`.
//! Future `PermissionController` (K-6) lands as a field here so the
//! auto-`Cancelled` policy flips to a real trust-store without
//! rippling through runtime + session call sites.

use agent_client_protocol::schema::{
    RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse, SessionNotification,
};
use tokio::sync::mpsc;

/// Payload pushed onto the per-session forwarding channel.
/// `Notification` boxed because `SessionNotification` is large
/// (`#[non_exhaustive]` with nested content blocks).
#[derive(Debug, Clone)]
pub enum ClientEvent {
    Notification(Box<SessionNotification>),
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
/// additions. A hardcoded match would silently diverge when the crate
/// adds a variant; stable Rust can't statically reject the omission
/// (only nightly's `non_exhaustive_omitted_patterns` lint can). If
/// serialisation ever fails we log loud and emit `"unknown"` — noisier
/// than fabricating a `Debug` string.
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

/// Per-session adapter: bundles the event sender + the tools the ACP
/// runtime closures invoke. Cloned into each `on_receive_*` closure.
#[derive(Debug, Clone)]
pub struct AcpClient {
    events: mpsc::UnboundedSender<ClientEvent>,
}

impl AcpClient {
    pub fn new(events: mpsc::UnboundedSender<ClientEvent>) -> Self {
        Self { events }
    }

    /// Forward a `SessionNotification` onto the per-session events
    /// channel. Closed receiver degrades to a trace line rather than
    /// an error — the actor has already finished its select loop.
    pub fn forward_notification(&self, notification: SessionNotification) {
        if self
            .events
            .send(ClientEvent::Notification(Box::new(notification)))
            .is_err()
        {
            tracing::trace!("acp::client: events channel closed, dropping notification");
        }
    }

    /// Handle a `session/request_permission` request. Emits an
    /// observability event for the webview, then auto-`Cancelled`.
    /// `PermissionController` (K-6) replaces the auto-cancel with a
    /// trust-store consult.
    pub fn request_permission(&self, req: &RequestPermissionRequest) -> RequestPermissionResponse {
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
        RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::schema::{
        PermissionOption, PermissionOptionId, PermissionOptionKind, RequestPermissionRequest, SessionId, ToolCallId,
        ToolCallUpdate,
    };

    #[test]
    fn permission_request_auto_denies() {
        let (tx, mut rx) = mpsc::unbounded_channel();
        let client = AcpClient::new(tx);
        let req = RequestPermissionRequest::new(
            SessionId::new("sess-1"),
            ToolCallUpdate::new(ToolCallId::new("tc-1"), Default::default()),
            vec![PermissionOption::new(
                PermissionOptionId::new("allow"),
                "Allow",
                PermissionOptionKind::AllowOnce,
            )],
        );
        let resp = client.request_permission(&req);
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
}
