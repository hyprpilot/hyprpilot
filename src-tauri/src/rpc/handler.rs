use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::broadcast;

use crate::adapters::Adapter;
use crate::config::Config;
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
/// handlers (`status/*`, `session/*`, `daemon/kill`) without a live
/// Tauri runtime. Handlers that need the window (`window/toggle`)
/// surface `RpcError::internal_error` when `app` is `None`.
pub struct HandlerCtx<'a> {
    pub app: Option<&'a tauri::AppHandle>,
    pub status: &'a StatusBroadcast,
    /// Shared adapter. `Option` so the unit-test harness can run
    /// without building a full adapter; production calls always pass
    /// `Some`. Typed as `Arc<dyn Adapter>` so handlers are adapter-agnostic
    /// — adding an HTTP transport does not touch this field.
    pub adapter: Option<Arc<dyn Adapter>>,
    /// Shared config snapshot. Read-only handlers (`config/profiles`,
    /// future `config/agents`) render from this directly; adapter
    /// methods never see it.
    pub config: Option<Arc<Config>>,
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
/// `status/get` + `status/subscribe` live under `"status"`;
/// `session/submit` + `session/cancel` + `session/info` under
/// `"session"`; `window/toggle` under `"window"`; `daemon/kill` under
/// `"daemon"`. Every K-239-era method uses the `namespace/name` form;
/// bare method names (e.g. `submit`, `toggle`) route to the empty
/// namespace, which has no registered handler and therefore returns
/// `-32601 method not found`.
///
/// Adding a namespace is a one-liner on the dispatcher (see
/// `RpcDispatcher::with_defaults`) — implement `RpcHandler`, register
/// it on the vector, done. Future namespaces: `agents/*`,
/// `permissions/*`.
#[async_trait]
pub trait RpcHandler: Send + Sync {
    /// Namespace before the `/` in every method this handler owns
    /// (`"session"`, `"window"`, `"daemon"`, `"status"`, ...). Dispatch
    /// matches this against the method prefix; no handler ships for the
    /// empty namespace, so bare method names are always `-32601`.
    fn namespace(&self) -> &'static str;

    /// Handle a single method call. Params are already JSON-typed; each
    /// handler is responsible for its own shape validation and for
    /// returning `RpcError::invalid_params` on mismatch.
    async fn handle(&self, method: &str, params: Value, ctx: HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError>;
}
