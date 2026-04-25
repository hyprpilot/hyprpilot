use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::{broadcast, mpsc, oneshot};

use crate::adapters::{AcpAdapter, Adapter};
use crate::config::Config;
use crate::rpc::protocol::{EventsNotifyParams, RequestId, RpcError, StatusResult};
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
    /// ACP-specific handle for handlers that need methods outside the
    /// generic `Adapter` trait — `profiles/list`, `agents/list` today.
    /// Production daemon always passes the same `Arc<AcpAdapter>` that
    /// sits behind `adapter`; tests can pass either or both.
    pub acp_adapter: Option<Arc<AcpAdapter>>,
    /// Shared config handle. Read-only at runtime — handlers
    /// (`config/profiles`, future `config/agents`) lock briefly to
    /// clone what they need. Config is static after daemon start;
    /// restart-to-change is the model.
    pub config: Option<Arc<RwLock<Config>>>,
    /// Request id of the in-flight call. Handlers read it for logging /
    /// tracing spans; unused by routing.
    #[allow(dead_code)]
    pub id: &'a RequestId,
    /// Whether this connection has already subscribed to `status/changed`.
    /// Threaded in by the server so `StatusHandler` can reject a second
    /// subscribe on the same socket with `-32600` (Thread 9).
    pub already_subscribed: bool,
    /// Subscription ids registered to this connection's `events/*`
    /// channel. Threaded in so `EventsHandler` can reject duplicate
    /// `events/unsubscribe` ids cleanly (responding with
    /// `{ unsubscribed: false }` rather than panicking on a vanished
    /// entry) and surface "not yours" without a server-side lookup.
    pub existing_event_subscription_ids: &'a [String],
    /// Lazy mpsc sender the events filter tasks push on. The server
    /// initialises this on first `events/subscribe`; subsequent
    /// subscribes on the same connection receive a clone via this
    /// reference. `None` means the connection has zero active events
    /// subscriptions and the server must mint a fresh `(tx, rx)` pair.
    pub events_tx: Option<&'a EventsConnectionTx>,
}

/// Outcome returned by a handler. Most calls are a plain `Reply(Value)`;
/// subscribers return one of the `*Subscribed` variants so the server
/// loop can pin the receiver onto the connection task and fan
/// notifications out as they arrive.
#[allow(clippy::large_enum_variant)]
pub enum HandlerOutcome {
    Reply(Value),
    /// `status/subscribe` — initial snapshot + a broadcast receiver that
    /// yields future state transitions. The server writes the snapshot
    /// as the call's JSON-RPC response, then drives the receiver in the
    /// connection's `select!` to push `status/changed` notifications.
    StatusSubscribed(Value, broadcast::Receiver<StatusResult>),
    /// `events/subscribe` — initial reply (`{ subscriptionId }`) + the
    /// data the connection loop needs to evict the subscription on
    /// `events/unsubscribe` (the cancel sender + an mpsc sender clone
    /// the spawned filter task pushes onto). The connection's
    /// `select!` drains a single shared mpsc receiver across every
    /// active subscription; per-subscription teardown drops the
    /// matching `EventsSubscription`, which fires the cancel +
    /// terminates the filter task on its next loop iteration.
    EventsSubscribed(Value, EventsSubscription),
}

/// Per-subscription handle the events/subscribe handler returns. The
/// connection's vec owns one per active subscription; dropping it
/// signals the cancel oneshot, which the filter task awaits in
/// `tokio::select!` alongside the broadcast recv — drop = task exits.
pub struct EventsSubscription {
    pub subscription_id: String,
    /// Held to keep the cancel signal armed; dropped only on
    /// `events/unsubscribe` or connection close. The matching
    /// receiver lives inside the filter task — never read directly.
    #[allow(dead_code)]
    pub cancel: oneshot::Sender<()>,
}

/// Bag the events handler hands back to the server: the per-subscription
/// drop-handle the connection vec stores, plus the mpsc sender the
/// filter task pushes notifications onto. Only the first
/// `events/subscribe` on a connection passes a fresh `(tx, rx)` pair;
/// subsequent subscribes receive a clone of the existing tx so every
/// subscription on a connection feeds the same outbound queue, which
/// the connection's `select!` drains in a single arm.
#[derive(Clone)]
pub struct EventsConnectionTx {
    pub tx: mpsc::Sender<EventsNotifyParams>,
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
