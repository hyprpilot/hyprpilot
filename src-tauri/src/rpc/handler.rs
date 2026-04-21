use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::broadcast;

use crate::rpc::protocol::{RequestId, RpcError, StatusResult};
use crate::rpc::status::StatusBroadcast;

/// Per-connection context handed to `RpcHandler::handle`. Groups the
/// shared app state, the current request id (for logging), and any
/// per-connection flags that a handler needs to consult (or mutate via
/// interior mutability on future additions).
///
/// Borrowing rather than cloning keeps the call hot — handlers can lean
/// on `ctx.app` and `ctx.status` without needing to juggle `Arc` clones.
///
/// `app` is `Option` so unit tests can drive routing and the pure-data
/// handlers (`status/*`, `submit`, `cancel`, `session-info`, `kill`)
/// without a live Tauri runtime. Handlers that need the window
/// (`toggle`) surface `RpcError::internal_error` when `app` is `None`.
pub struct HandlerCtx<'a> {
    pub app: Option<&'a tauri::AppHandle>,
    pub status: &'a StatusBroadcast,
    /// Request id of the in-flight call. Handlers read it for logging /
    /// tracing spans; unused by routing.
    #[allow(dead_code)]
    pub id: &'a RequestId,
    /// Whether this connection has already subscribed to `status/changed`.
    /// Threaded in by the server so `StatusHandler` can reject a second
    /// subscribe on the same socket with `-32600` (Thread 9).
    pub already_subscribed: bool,
}

/// Outcome returned by a handler. Most calls are a plain `Reply(Value)`;
/// `status/subscribe` returns `Subscribed(snapshot, receiver)` so the
/// server loop can pin the receiver onto the connection task and fan
/// `status/changed` notifications out as they arrive.
#[allow(clippy::large_enum_variant)]
pub enum HandlerOutcome {
    Reply(Value),
    /// Initial snapshot + a broadcast receiver that yields future state
    /// transitions. The server writes the snapshot as the call's JSON-RPC
    /// response, then drives the receiver in the connection's `select!`.
    Subscribed(Value, broadcast::Receiver<StatusResult>),
}

/// A unit of RPC work, keyed by the namespace prefix of its method names.
///
/// Handlers match by the namespace before the `/` in a method name:
/// `status/get` + `status/subscribe` live under the `"status"` namespace;
/// un-namespaced methods (`submit`, `toggle`, `kill`, ...) belong to
/// `CoreHandler` whose `namespace()` returns `""`.
///
/// Adding a namespace is a one-liner on the dispatcher (see
/// `RpcDispatcher::with_defaults`) — implement `RpcHandler`, register it
/// on the vector, done. Future namespaces: `session/*`, `window/*`,
/// `daemon/*`, `agent/*`.
#[async_trait]
pub trait RpcHandler: Send + Sync {
    /// Namespace before the `/`. `""` (empty) means un-namespaced (e.g.
    /// `toggle`, `submit`, legacy shortnames from the scaffold).
    fn namespace(&self) -> &'static str;

    /// Handle a single method call. Params are already JSON-typed; each
    /// handler is responsible for its own shape validation and for
    /// returning `RpcError::invalid_params` on mismatch.
    async fn handle(&self, method: &str, params: Value, ctx: HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError>;
}
