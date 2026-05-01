pub mod handler;
pub mod handlers;
pub mod protocol;
pub mod server;
pub mod status;

use serde_json::Value;

pub use handler::{HandlerCtx, HandlerOutcome, RpcHandler};
pub use handlers::{
    DaemonHandler, DiagHandler, InstancesHandler, OverlayHandler, PermissionsHandler, PromptsHandler, StatusHandler,
};
pub use server::{handle_connection, RpcState};
pub use status::StatusBroadcast;

use crate::rpc::protocol::RpcError;

/// Registry of every `RpcHandler` implementation wired into the daemon.
///
/// Dispatch is a single pass over `handlers`: pull the namespace prefix
/// from the method name (`"status/get"` → `"status"`), find the first
/// handler whose `namespace()` matches, and delegate. Methods without
/// a `/` route to the empty namespace, which has no registered
/// handler — bare method names (`submit`, `toggle`, `kill`, etc.) all
/// return `-32601 method not found`. That's load-bearing: the K-239
/// renames are intentionally breaking and bare names must stay dead.
///
/// Extending the RPC surface means implementing `RpcHandler` and pushing
/// a new instance onto the vector in `with_defaults`.
///
/// Wire surface today (7 namespaces, ~18 verbs):
/// - `daemon/{kill, status, version, shutdown}` — operator surface.
/// - `diag/snapshot` — read-only structural snapshot.
/// - `instances/*` — live process management for scripting.
/// - `overlay/{present, hide, toggle}` — hyprland-bind surface.
/// - `permissions/{pending, respond}` — script-driven permission resolution.
/// - `prompts/{send, cancel}` — per-instance scripting.
/// - `status/{get, subscribe}` — waybar.
///
/// The webview goes through Tauri commands (in `adapters/commands.rs`),
/// not this RPC surface. RPC is the operator / scripting transport.
pub struct RpcDispatcher {
    handlers: Vec<Box<dyn RpcHandler>>,
}

impl RpcDispatcher {
    pub fn with_defaults() -> Self {
        Self {
            handlers: vec![
                Box::new(OverlayHandler),
                Box::new(DaemonHandler),
                Box::new(DiagHandler),
                Box::new(StatusHandler),
                Box::new(InstancesHandler),
                Box::new(PromptsHandler),
                Box::new(PermissionsHandler),
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
    use crate::adapters::{AcpAdapter, Adapter};
    use crate::config::Config;
    use crate::rpc::protocol::RequestId;
    use crate::rpc::status::StatusBroadcast;
    use serde_json::json;

    /// Dispatch one call through the registry with `ctx.app = None`.
    /// Handlers that need the Tauri app surface `-32603`; everything
    /// else drives its handler to completion.
    async fn call(dispatcher: &RpcDispatcher, broadcast: &StatusBroadcast, method: &str, params: Value) -> Value {
        let id = RequestId::Number(1);
        let config = Arc::new(std::sync::RwLock::new(Config::default()));
        let acp = Arc::new(AcpAdapter::new(Config::default(), Arc::new(StatusBroadcast::new(true))));
        let adapter: Arc<dyn Adapter> = acp.clone();
        let ctx = HandlerCtx {
            app: None,
            status: broadcast,
            adapter,
            config: Some(config),
            id: &id,
            already_subscribed: false,
            started_at: None,
            socket_path: None,
        };
        match dispatcher.dispatch(method, params, ctx).await {
            Ok(HandlerOutcome::Reply(v)) => v,
            Ok(HandlerOutcome::StatusSubscribed(v, _rx)) => v,
            Err(e) => json!({ "code": e.code, "message": e.message}),
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
    async fn dispatch_routes_instances_list_to_instances_handler() {
        let dispatcher = RpcDispatcher::with_defaults();
        let broadcast = StatusBroadcast::new(true);
        let v = call(&dispatcher, &broadcast, "instances/list", Value::Null).await;
        assert_eq!(v["instances"], json!([]));
    }

    #[tokio::test]
    async fn dispatch_routes_instances_focus_unknown_id_is_invalid_params() {
        let dispatcher = RpcDispatcher::with_defaults();
        let broadcast = StatusBroadcast::new(true);
        let v = call(
            &dispatcher,
            &broadcast,
            "instances/focus",
            json!({ "id": "550e8400-e29b-41d4-a716-446655440000" }),
        )
        .await;
        assert_eq!(v["code"], -32602, "unknown instance id must be invalid_params: {v}");
    }

    #[tokio::test]
    async fn dispatch_routes_instances_info_missing_id_is_invalid_params() {
        let dispatcher = RpcDispatcher::with_defaults();
        let broadcast = StatusBroadcast::new(true);
        let v = call(&dispatcher, &broadcast, "instances/info", Value::Null).await;
        assert_eq!(v["code"], -32602);
    }

    #[tokio::test]
    async fn dispatch_unknown_instances_verb_is_method_not_found() {
        let dispatcher = RpcDispatcher::with_defaults();
        let broadcast = StatusBroadcast::new(true);
        let v = call(&dispatcher, &broadcast, "instances/bogus", Value::Null).await;
        assert_eq!(v["code"], -32601);
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

    /// Dropped namespaces (`session/*`, `agents/*`, `commands/*`,
    /// `completion/*`, `config/*`, `events/*`, `mcps/*`, `models/*`,
    /// `modes/*`, `profiles/*`, `sessions/*`, `skills/*`, `window/*`)
    /// all return `-32601`. Webview consumers go through Tauri commands;
    /// hyprland-bind users move from `window/toggle` to
    /// `overlay/toggle`. Completion is webview-only — no socket
    /// scripting story exists for it.
    #[tokio::test]
    async fn dispatch_pruned_namespaces_are_method_not_found() {
        let dispatcher = RpcDispatcher::with_defaults();
        let broadcast = StatusBroadcast::new(true);
        for method in [
            "session/submit",
            "session/cancel",
            "session/info",
            "agents/list",
            "commands/list",
            "completion/query",
            "completion/resolve",
            "completion/cancel",
            "config/profiles",
            "events/subscribe",
            "events/unsubscribe",
            "mcps/list",
            "models/list",
            "modes/list",
            "profiles/list",
            "sessions/list",
            "skills/list",
            "window/toggle",
        ] {
            let v = call(&dispatcher, &broadcast, method, Value::Null).await;
            assert_eq!(v["code"], -32601, "pruned {method} must be method_not_found");
        }
    }

    /// Bare method names (the pre-K-239 scaffold) all return
    /// `-32601 method_not_found`. No backwards-compat layer.
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
        let config = Arc::new(std::sync::RwLock::new(Config::default()));
        let acp = Arc::new(AcpAdapter::new(Config::default(), Arc::new(StatusBroadcast::new(true))));
        let adapter: Arc<dyn Adapter> = acp.clone();
        let ctx = HandlerCtx {
            app: None,
            status: &broadcast,
            adapter,
            config: Some(config),
            id: &id,
            already_subscribed: true,
            started_at: None,
            socket_path: None,
        };
        let res = dispatcher.dispatch("status/subscribe", Value::Null, ctx).await;
        match res {
            Err(e) => assert_eq!(e.code, -32600),
            Ok(_) => panic!("double subscribe must fail"),
        }
    }
}
