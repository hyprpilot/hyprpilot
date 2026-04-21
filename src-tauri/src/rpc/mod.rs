pub mod handler;
pub mod handlers;
pub mod protocol;
pub mod server;
pub mod status;

use serde_json::Value;

pub use handler::{HandlerCtx, HandlerOutcome, RpcHandler};
pub use handlers::{CoreHandler, StatusHandler};
pub use server::{handle_connection, RpcState};
pub use status::StatusBroadcast;

use crate::rpc::protocol::RpcError;

/// Registry of every `RpcHandler` implementation wired into the daemon.
///
/// Dispatch is a single pass over `handlers`: pull the namespace prefix
/// from the method name (`"status/get"` → `"status"`, `"toggle"` → `""`),
/// find the first handler whose `namespace()` matches, and delegate.
/// Unknown namespaces produce `-32601 method not found`.
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
    /// - `CoreHandler` (namespace `""`): `submit`, `cancel`, `toggle`,
    ///   `kill`, `session-info`.
    /// - `StatusHandler` (namespace `"status"`): `status/get`,
    ///   `status/subscribe`.
    pub fn with_defaults() -> Self {
        Self {
            handlers: vec![Box::new(CoreHandler), Box::new(StatusHandler)],
        }
    }

    /// Look up the handler by the namespace prefix of `method` and
    /// delegate. The prefix is the text before the first `/`; methods
    /// without a `/` route to the empty-namespace handler.
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
    use super::*;
    use crate::rpc::protocol::RequestId;
    use crate::rpc::status::StatusBroadcast;
    use serde_json::json;

    /// Dispatch one call through the registry with `ctx.app = None`.
    /// Handlers that need the Tauri app (only `toggle` today) surface
    /// `-32603`; everything else drives its handler to completion.
    async fn call(dispatcher: &RpcDispatcher, broadcast: &StatusBroadcast, method: &str, params: Value) -> Value {
        let id = RequestId::Number(1);
        let ctx = HandlerCtx {
            app: None,
            status: broadcast,
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

    #[tokio::test]
    async fn dispatch_routes_submit_to_core_handler() {
        let dispatcher = RpcDispatcher::with_defaults();
        let broadcast = StatusBroadcast::new(true);
        let v = call(&dispatcher, &broadcast, "submit", json!({ "text": "hi" })).await;
        assert_eq!(v["accepted"], true);
        assert_eq!(v["text"], "hi");
    }

    #[tokio::test]
    async fn dispatch_routes_cancel_to_core_handler() {
        let dispatcher = RpcDispatcher::with_defaults();
        let broadcast = StatusBroadcast::new(true);
        let v = call(&dispatcher, &broadcast, "cancel", Value::Null).await;
        assert_eq!(v["cancelled"], false);
    }

    #[tokio::test]
    async fn dispatch_routes_session_info_to_core_handler() {
        let dispatcher = RpcDispatcher::with_defaults();
        let broadcast = StatusBroadcast::new(true);
        let v = call(&dispatcher, &broadcast, "session-info", Value::Null).await;
        assert_eq!(v["sessions"], json!([]));
    }

    #[tokio::test]
    async fn dispatch_toggle_without_app_is_internal_error() {
        let dispatcher = RpcDispatcher::with_defaults();
        let broadcast = StatusBroadcast::new(true);
        let v = call(&dispatcher, &broadcast, "toggle", Value::Null).await;
        assert_eq!(v["code"], -32603);
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
    async fn dispatch_submit_missing_text_is_invalid_params() {
        let dispatcher = RpcDispatcher::with_defaults();
        let broadcast = StatusBroadcast::new(true);
        let v = call(&dispatcher, &broadcast, "submit", Value::Null).await;
        assert_eq!(v["code"], -32602);
    }

    #[tokio::test]
    async fn dispatch_status_subscribe_twice_is_invalid_request() {
        let dispatcher = RpcDispatcher::with_defaults();
        let broadcast = StatusBroadcast::new(true);
        let id = RequestId::Number(2);
        let ctx = HandlerCtx {
            app: None,
            status: &broadcast,
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
