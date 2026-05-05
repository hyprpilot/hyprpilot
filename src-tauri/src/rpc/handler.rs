use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time::Instant;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::broadcast;

use crate::adapters::Adapter;
use crate::config::Config;
use crate::rpc::protocol::{RpcError, StatusResult};
use crate::rpc::status::StatusBroadcast;

/// Per-connection context handed to `RpcHandler::handle`. Groups the
/// shared app state, the current request id (for logging), and any
/// per-connection flags that a handler needs to consult.
///
/// Borrowing rather than cloning keeps the call hot — handlers can lean
/// on `ctx.app` and `ctx.status` without needing to juggle `Arc` clones.
///
/// `app` is `Option` so unit tests can drive routing and the pure-data
/// handlers (`status/*`, `daemon/kill`) without a live Tauri runtime.
/// Handlers that need the window (`overlay/*`) surface
/// `RpcError::internal_error` when `app` is `None`.
pub struct HandlerCtx<'a> {
    pub app: Option<&'a tauri::AppHandle>,
    pub status: &'a StatusBroadcast,
    /// Shared adapter. Mandatory — every production caller passes one,
    /// and tests construct one anyway. Typed as `Arc<dyn Adapter>` so
    /// handlers stay transport-agnostic; the trait is the single
    /// abstraction layer that an HTTP transport will plug into.
    pub adapter: Arc<dyn Adapter>,
    /// Shared config handle. Read-only at runtime — handlers lock
    /// briefly to clone what they need. Config is static after daemon
    /// start; restart-to-change is the model.
    pub config: Option<Arc<RwLock<Config>>>,
    /// Shared skills registry. `daemon/reload` calls
    /// `SkillsRegistry::reload()` to rescan on-disk catalogues.
    /// `None` in unit tests where reload isn't exercised.
    pub skills: Option<Arc<crate::skills::SkillsRegistry>>,
    /// Shared MCP registry. `daemon/reload` reads its `list().len()`
    /// for the response payload. `None` in unit tests.
    pub mcps: Option<Arc<crate::mcp::MCPsRegistry>>,
    /// Whether this connection has already subscribed to `status/changed`.
    /// Threaded in by the server so `StatusHandler` can reject a second
    /// subscribe on the same socket with `-32600`.
    pub already_subscribed: bool,
    /// Process start time. `daemon/status` reports `uptimeSecs` against
    /// this. `None` in unit tests where uptime is irrelevant.
    pub started_at: Option<Instant>,
    /// Resolved unix socket path. Surfaced by `daemon/status` +
    /// `diag/snapshot`. `None` in unit tests.
    pub socket_path: Option<&'a Path>,
}

/// Outcome returned by a handler. Most calls are a plain `Reply(Value)`;
/// `status/subscribe` returns the snapshot + a broadcast receiver so
/// the server can pin the receiver onto the connection task and fan
/// `status/changed` notifications out as they arrive. The receiver
/// boxes to keep `Reply` (the hot path) compact.
pub enum HandlerOutcome {
    Reply(Value),
    /// `status/subscribe` — initial snapshot + a broadcast receiver
    /// that yields future state transitions. The server writes the
    /// snapshot as the call's JSON-RPC response, then drives the
    /// receiver in the connection's `select!` to push `status/changed`
    /// notifications.
    StatusSubscribed(Value, Box<broadcast::Receiver<StatusResult>>),
}

/// A unit of RPC work, keyed by the namespace prefix of its method names.
///
/// Handlers match by the namespace before the `/` in a method name:
/// `status/get` + `status/subscribe` live under `"status"`;
/// `daemon/kill` under `"daemon"`. Bare method names route to the
/// empty namespace, which has no registered handler — they always
/// return `-32601 method not found`.
///
/// Adding a namespace is a one-liner on the dispatcher (see
/// `RpcDispatcher::with_defaults`).
#[async_trait]
pub trait RpcHandler: Send + Sync {
    /// Namespace before the `/` in every method this handler owns
    /// (`"status"`, `"daemon"`, ...). Dispatch matches this against
    /// the method prefix; no handler ships for the empty namespace,
    /// so bare method names are always `-32601`.
    fn namespace(&self) -> &'static str;

    /// Handle a single method call. Params are already JSON-typed; each
    /// handler is responsible for its own shape validation and for
    /// returning `RpcError::invalid_params` on mismatch.
    async fn handle(&self, method: &str, params: Value, ctx: HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError>;
}
