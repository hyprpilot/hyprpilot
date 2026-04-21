use async_trait::async_trait;
use serde_json::{json, Value};

use crate::rpc::handler::{HandlerCtx, HandlerOutcome, RpcHandler};
use crate::rpc::protocol::RpcError;

/// `daemon/*` namespace — daemon lifecycle.
///
/// Today only `daemon/kill` ships; future methods under this
/// namespace include `daemon/status`, `daemon/reload`. The actual
/// `app.exit(0)` call lives in `server::handle_connection` — this
/// handler just returns the `exiting: true` reply that the server
/// writes before shutting the process down.
pub struct DaemonHandler;

#[async_trait]
impl RpcHandler for DaemonHandler {
    fn namespace(&self) -> &'static str {
        "daemon"
    }

    async fn handle(&self, method: &str, _params: Value, _ctx: HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
        match method {
            "daemon/kill" => Ok(HandlerOutcome::Reply(json!({ "exiting": true }))),
            other => Err(RpcError::method_not_found(other)),
        }
    }
}
