use async_trait::async_trait;
use serde_json::{json, Value};

use crate::rpc::handler::{HandlerCtx, HandlerOutcome, RpcHandler};
use crate::rpc::protocol::RpcError;

/// `daemon/*` namespace — daemon lifecycle.
///
/// Today only `daemon/kill` ships; future methods under this
/// namespace include `daemon/status`, `daemon/reload`.
///
/// `daemon/kill` returns `{"killed": true}`. The actual `app.exit(0)`
/// call lives in `server::handle_connection`, which inspects the
/// result payload for that marker after the response has been
/// flushed to the peer. Keeping the signal *in the result*
/// (instead of a side-channel flag threaded through the dispatcher
/// tuple) means the client sees the same thing the server reads,
/// and any future "respond-then-shut-down" handlers just emit
/// `{"killed": true}` — no new plumbing.
pub struct DaemonHandler;

#[async_trait]
impl RpcHandler for DaemonHandler {
    fn namespace(&self) -> &'static str {
        "daemon"
    }

    async fn handle(&self, method: &str, _params: Value, _ctx: HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
        match method {
            "daemon/kill" => Ok(HandlerOutcome::Reply(json!({ "killed": true }))),
            other => Err(RpcError::method_not_found(other)),
        }
    }
}
