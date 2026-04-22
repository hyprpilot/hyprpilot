pub mod handler;
pub mod handlers;
pub mod protocol;
pub mod server;
pub mod status;

use serde_json::Value;

pub use handler::{HandlerCtx, HandlerOutcome, RpcHandler};
pub use handlers::{DaemonHandler, SessionHandler, StatusHandler, WindowHandler};
pub use server::{handle_connection, RpcState};
pub use status::StatusBroadcast;

use crate::rpc::protocol::RpcError;

/// Registry of every `RpcHandler` implementation wired into the daemon.
///
/// Dispatch is a single pass over `handlers`: pull the namespace prefix
/// from the method name (`"status/get"` → `"status"`, `"window/toggle"`
/// → `"window"`), find the first handler whose `namespace()` matches,
/// and delegate. Methods without a `/` route to the empty-namespace
/// handler; no handler ships for the empty namespace today, so any
/// bare method name (`submit`, `toggle`, `kill`, etc.) surfaces as
/// `-32601 method not found`. That's load-bearing: the K-239 renames
/// are intentionally breaking and bare names must stay dead.
///
/// Extending the RPC surface means implementing `RpcHandler` and pushing
/// a new instance onto the vector in `with_defaults`. No other file
/// changes required — `server::handle_connection` is namespace-agnostic.
pub struct RpcDispatcher {
    handlers: Vec<Box<dyn RpcHandler>>,
}

impl RpcDispatcher {
    /// Build the default registry. Every RPC method the daemon implements
    /// today lives in one of these handlers:
    ///
    /// - `SessionHandler` (namespace `"session"`): `session/submit`,
    ///   `session/cancel`, `session/info`.
    /// - `WindowHandler` (namespace `"window"`): `window/toggle`.
    /// - `DaemonHandler` (namespace `"daemon"`): `daemon/kill`.
    /// - `StatusHandler` (namespace `"status"`): `status/get`,
    ///   `status/subscribe`.
    pub fn with_defaults() -> Self {
        Self {
            handlers: vec![
                Box::new(SessionHandler),
                Box::new(WindowHandler),
                Box::new(DaemonHandler),
                Box::new(StatusHandler),
            ],
        }
    }

    /// Look up the handler by the namespace prefix of `method` and
    /// delegate. The prefix is the text before the first `/`; methods
    /// without a `/` fall into the empty-namespace bucket, which has
    /// no registered handler (so they return `-32601`).
    pub async fn dispatch(&self, method: &str, params: Value, ctx: HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
        let ns = method.split_once('/').map(|(n, _)| n).unwrap_or("");
        let handler = self
            .handlers
            .iter()
            .find(|h| h.namespace() == ns)
            .ok_or_else(|| RpcError::method_not_found(method))?;
        handler.handle(method, params, ctx).await
    }
}

impl Default for RpcDispatcher {
    fn default() -> Self {
        Self::with_defaults()
    }
}

#[cfg(test)]
mod dispatcher_tests {
    use std::sync::Arc;

    use super::*;
    use crate::acp::AcpSessions;
    use crate::config::AgentsConfig;
    use crate::rpc::protocol::RequestId;
    use crate::rpc::status::StatusBroadcast;
    use serde_json::json;

    /// Dispatch one call through the registry with `ctx.app = None`.
    /// Handlers that need the Tauri app (only `window/toggle` today)
    /// surface `-32603`; everything else drives its handler to
    /// completion.
    async fn call(dispatcher: &RpcDispatcher, broadcast: &StatusBroadcast, method: &str, params: Value) -> Value {
        let id = RequestId::Number(1);
        let sessions = Arc::new(AcpSessions::new(
            AgentsConfig::default(),
            Arc::new(StatusBroadcast::new(true)),
        ));
        let ctx = HandlerCtx {
            app: None,
            status: broadcast,
            sessions: Some(sessions),
            id: &id,
            already_subscribed: false,
        };
        match dispatcher.dispatch(method, params, ctx).await {
            Ok(HandlerOutcome::Reply(v)) => v,
            Ok(HandlerOutcome::Subscribed(v, _rx)) => v,
            Err(e) => json!({ "code": e.code, "message": e.message }),
        }
    }

    #[tokio::test]
    async fn dispatch_routes_status_get_to_status_handler() {
        let dispatcher = RpcDispatcher::with_defaults();
        let broadcast = StatusBroadcast::new(true);
        let v = call(&dispatcher, &broadcast, "status/get", Value::Null).await;
        assert_eq!(v["state"], "idle");
        assert_eq!(v["visible"], true);
    }

    #[tokio::test]
    async fn dispatch_routes_status_subscribe_returns_snapshot() {
        let dispatcher = RpcDispatcher::with_defaults();
        let broadcast = StatusBroadcast::new(true);
        let v = call(&dispatcher, &broadcast, "status/subscribe", Value::Null).await;
        assert_eq!(v["state"], "idle");
    }

    /// Empty `AgentsConfig` — no `[agent] default`, no registry
    /// entries. `session/submit` must return `-32602 invalid_params`
    /// because there's no way to resolve an agent to spawn.
    #[tokio::test]
    async fn dispatch_routes_session_submit_to_session_handler() {
        let dispatcher = RpcDispatcher::with_defaults();
        let broadcast = StatusBroadcast::new(true);
        let v = call(&dispatcher, &broadcast, "session/submit", json!({ "text": "hi" })).await;
        assert_eq!(v["code"], -32602, "empty config must reject with invalid_params: {v}");
    }

    #[tokio::test]
    async fn dispatch_routes_session_cancel_to_session_handler() {
        let dispatcher = RpcDispatcher::with_defaults();
        let broadcast = StatusBroadcast::new(true);
        let v = call(&dispatcher, &broadcast, "session/cancel", Value::Null).await;
        assert_eq!(v["code"], -32602, "empty config rejects cancel: {v}");
    }

    #[tokio::test]
    async fn dispatch_routes_session_info_to_session_handler() {
        let dispatcher = RpcDispatcher::with_defaults();
        let broadcast = StatusBroadcast::new(true);
        let v = call(&dispatcher, &broadcast, "session/info", Value::Null).await;
        assert_eq!(v["sessions"], json!([]));
    }

    #[tokio::test]
    async fn dispatch_window_toggle_without_app_is_internal_error() {
        let dispatcher = RpcDispatcher::with_defaults();
        let broadcast = StatusBroadcast::new(true);
        let v = call(&dispatcher, &broadcast, "window/toggle", Value::Null).await;
        assert_eq!(v["code"], -32603);
    }

    #[tokio::test]
    async fn dispatch_routes_daemon_kill_to_daemon_handler() {
        let dispatcher = RpcDispatcher::with_defaults();
        let broadcast = StatusBroadcast::new(true);
        let v = call(&dispatcher, &broadcast, "daemon/kill", Value::Null).await;
        assert_eq!(v["killed"], true);
    }

    #[tokio::test]
    async fn dispatch_unknown_namespace_is_method_not_found() {
        let dispatcher = RpcDispatcher::with_defaults();
        let broadcast = StatusBroadcast::new(true);
        let v = call(&dispatcher, &broadcast, "foobar/baz", Value::Null).await;
        assert_eq!(v["code"], -32601);
    }

    #[tokio::test]
    async fn dispatch_unknown_method_in_known_namespace_is_method_not_found() {
        let dispatcher = RpcDispatcher::with_defaults();
        let broadcast = StatusBroadcast::new(true);
        let v = call(&dispatcher, &broadcast, "status/bogus", Value::Null).await;
        assert_eq!(v["code"], -32601);
    }

    #[tokio::test]
    async fn dispatch_session_submit_missing_text_is_invalid_params() {
        let dispatcher = RpcDispatcher::with_defaults();
        let broadcast = StatusBroadcast::new(true);
        let v = call(&dispatcher, &broadcast, "session/submit", Value::Null).await;
        assert_eq!(v["code"], -32602);
    }

    /// Every bare method name from the pre-K-239 scaffold must return
    /// `-32601 method_not_found` after the rename. No backwards-compat
    /// layer — the contract is broken intentionally.
    #[tokio::test]
    async fn dispatch_bare_legacy_method_names_are_method_not_found() {
        let dispatcher = RpcDispatcher::with_defaults();
        let broadcast = StatusBroadcast::new(true);
        for method in ["submit", "cancel", "toggle", "kill", "session-info"] {
            let v = call(&dispatcher, &broadcast, method, Value::Null).await;
            assert_eq!(v["code"], -32601, "bare {method} must be method_not_found");
        }
    }

    #[tokio::test]
    async fn dispatch_status_subscribe_twice_is_invalid_request() {
        let dispatcher = RpcDispatcher::with_defaults();
        let broadcast = StatusBroadcast::new(true);
        let id = RequestId::Number(2);
        let sessions = Arc::new(AcpSessions::new(
            AgentsConfig::default(),
            Arc::new(StatusBroadcast::new(true)),
        ));
        let ctx = HandlerCtx {
            app: None,
            status: &broadcast,
            sessions: Some(sessions),
            id: &id,
            already_subscribed: true,
        };
        let res = dispatcher.dispatch("status/subscribe", Value::Null, ctx).await;
        match res {
            Err(e) => assert_eq!(e.code, -32600),
            Ok(_) => panic!("double subscribe must fail"),
        }
    }
}
