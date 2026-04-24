use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

use crate::rpc::handler::{HandlerCtx, HandlerOutcome, RpcHandler};
use crate::rpc::protocol::RpcError;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InstanceIdOnly {
    instance_id: String,
}

/// `commands/*` namespace: `commands/list` today. Instance-scoped;
/// routes by `instance_id` membership. Past the check the handler
/// `unimplemented!()`s — the ACP `available_commands` surface ships
/// as a `SessionUpdate` stream variant, not a request, so surfacing
/// it requires `AcpInstance` to cache the last-seen update. Ref
/// K-251.
pub struct CommandsHandler;

#[async_trait]
impl RpcHandler for CommandsHandler {
    fn namespace(&self) -> &'static str {
        "commands"
    }

    async fn handle(&self, method: &str, params: Value, ctx: HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
        let adapter = ctx
            .acp_adapter
            .as_ref()
            .ok_or_else(|| RpcError::internal_error("acp adapter not in managed state"))?;

        match method {
            "commands/list" => {
                let InstanceIdOnly { instance_id } = serde_json::from_value(params)
                    .map_err(|err| RpcError::invalid_params(format!("commands/list params: {err}")))?;
                let _ = adapter.contains_instance(&instance_id).await?;
                Err(RpcError::internal_error("commands/list not implemented — ref K-251"))
            }
            other => Err(RpcError::method_not_found(other)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::{AcpAdapter, Adapter, DefaultPermissionController, PermissionController};
    use crate::config::Config;
    use crate::rpc::handler::HandlerCtx;
    use crate::rpc::protocol::RequestId;
    use crate::rpc::status::StatusBroadcast;
    use serde_json::json;
    use std::sync::{Arc, RwLock};

    async fn dispatch(method: &str, params: Value) -> Value {
        let shared = Arc::new(RwLock::new(Config::default()));
        let adapter = Arc::new(AcpAdapter::with_shared_config(
            shared.clone(),
            Arc::new(StatusBroadcast::new(true)),
            Arc::new(DefaultPermissionController::new()) as Arc<dyn PermissionController>,
        ));
        let status = StatusBroadcast::new(true);
        let id = RequestId::Number(1);
        let dyn_adapter: Arc<dyn Adapter> = adapter.clone();
        let ctx = HandlerCtx {
            app: None,
            status: &status,
            adapter: Some(dyn_adapter),
            acp_adapter: Some(adapter),
            config: Some(shared),
            id: &id,
            already_subscribed: false,
        };
        match CommandsHandler.handle(method, params, ctx).await {
            Ok(HandlerOutcome::Reply(v)) => v,
            Ok(HandlerOutcome::Subscribed(v, _)) => v,
            Err(err) => json!({ "code": err.code, "message": err.message }),
        }
    }

    #[tokio::test]
    async fn commands_list_missing_instance_id_is_invalid_params() {
        let v = dispatch("commands/list", json!({})).await;
        assert_eq!(v["code"], -32602);
    }

    #[tokio::test]
    async fn commands_list_unknown_instance_id_is_invalid_params() {
        let v = dispatch(
            "commands/list",
            json!({ "instance_id": "550e8400-e29b-41d4-a716-446655440000" }),
        )
        .await;
        assert_eq!(v["code"], -32602);
    }

    #[tokio::test]
    async fn commands_unknown_verb_is_method_not_found() {
        let v = dispatch("commands/bogus", Value::Null).await;
        assert_eq!(v["code"], -32601);
    }
}
