use async_trait::async_trait;
use serde_json::{json, Value};

use crate::rpc::handler::{HandlerCtx, HandlerOutcome, RpcHandler};
use crate::rpc::protocol::RpcError;

/// `daemon/*` namespace (daemon lifecycle). `daemon/kill` returns
/// `{"killed": true}`; the server inspects that marker after the
/// flush and runs `daemon::shutdown`.
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
