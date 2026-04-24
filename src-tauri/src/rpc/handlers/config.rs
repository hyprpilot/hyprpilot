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
        let config = ctx
            .config
            .as_ref()
            .ok_or_else(|| RpcError::internal_error("config not in managed state"))?;

        match method {
            "config/profiles" => {
                let default_profile = config.agents.agent.default_profile.as_deref();
                let profiles: Vec<Value> = config
                    .profiles
                    .iter()
                    .map(|p| {
                        json!({
                            "id": p.id,
                            "agent": p.agent,
                            "model": p.model,
                            "has_prompt": p.system_prompt.is_some() || p.system_prompt_file.is_some(),
                            "is_default": default_profile == Some(p.id.as_str()),
                        })
                    })
                    .collect();
                Ok(HandlerOutcome::Reply(json!({ "profiles": profiles })))
            }
            other => Err(RpcError::method_not_found(other)),
        }
    }
}
