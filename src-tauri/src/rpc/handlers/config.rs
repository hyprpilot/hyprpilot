use async_trait::async_trait;
use serde_json::{json, Value};

use crate::rpc::handler::{HandlerCtx, HandlerOutcome, RpcHandler};
use crate::rpc::protocol::RpcError;

/// `config/*` namespace — `config/profiles` today. Exposes the
/// read-only slice of `Config` the chat UI + ctl need to render the
/// profile picker (K-246 consumes this; K-242 only ships the wire
/// surface).
pub struct ConfigHandler;

#[async_trait]
impl RpcHandler for ConfigHandler {
    fn namespace(&self) -> &'static str {
        "config"
    }

    async fn handle(&self, method: &str, _params: Value, ctx: HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
        let sessions = ctx
            .sessions
            .as_ref()
            .ok_or_else(|| RpcError::internal_error("AcpInstances not in managed state"))?;

        match method {
            "config/profiles" => Ok(HandlerOutcome::Reply(json!({
                "profiles": sessions.list_profiles(),
            }))),
            other => Err(RpcError::method_not_found(other)),
        }
    }
}
