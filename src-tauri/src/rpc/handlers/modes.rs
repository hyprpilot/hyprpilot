use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

use crate::rpc::handler::{HandlerCtx, HandlerOutcome, RpcHandler};
use crate::rpc::handlers::util::InstanceIdOnly;
use crate::rpc::protocol::RpcError;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SetModeParams {
    instance_id: String,
    // Consumed when ACP `session/set_mode` wiring lands (K-251). Kept
    // strict with `deny_unknown_fields` now so clients can send it
    // today without a second contract change.
    #[allow(dead_code)]
    mode_id: String,
}

/// `modes/*` namespace: `modes/list`, `modes/set`. Both are
/// instance-scoped. `-32602` when the id is missing / malformed /
/// not in the live registry. Beyond the membership check the
/// handler `unimplemented!()`s — `AcpInstance` doesn't cache the
/// available-modes list yet (K-251 landed the `mode` carry but not
/// the `available_modes` capability cache) and the runtime actor
/// doesn't accept a `SetMode` command yet either. Both are
/// follow-ups tracked under K-251.
pub struct ModesHandler;

#[async_trait]
impl RpcHandler for ModesHandler {
    fn namespace(&self) -> &'static str {
        "modes"
    }

    async fn handle(&self, method: &str, params: Value, ctx: HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
        let adapter = ctx
            .acp_adapter
            .as_ref()
            .ok_or_else(|| RpcError::internal_error("acp adapter not in managed state"))?;

        match method {
            "modes/list" => {
                let InstanceIdOnly { instance_id } = serde_json::from_value(params)
                    .map_err(|err| RpcError::invalid_params(format!("modes/list params: {err}")))?;
                let _ = adapter.contains_instance(&instance_id).await?;
                Err(RpcError::internal_error("modes/list not implemented — ref K-251"))
            }
            "modes/set" => {
                let SetModeParams {
                    instance_id,
                    mode_id: _,
                } = serde_json::from_value(params)
                    .map_err(|err| RpcError::invalid_params(format!("modes/set params: {err}")))?;
                let _ = adapter.contains_instance(&instance_id).await?;
                Err(RpcError::internal_error("modes/set not implemented — ref K-251"))
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
            started_at: None,
            socket_path: None,
            config_load_context: None,
            skills: None,
            mcps: None,
            existing_event_subscription_ids: &[],
            events_tx: None,
        };
        match ModesHandler.handle(method, params, ctx).await {
            Ok(HandlerOutcome::Reply(v)) => v,
            Ok(HandlerOutcome::StatusSubscribed(v, _)) => v,
            Ok(HandlerOutcome::EventsSubscribed(v, _)) => v,
            Err(err) => json!({ "code": err.code, "message": err.message }),
        }
    }

    #[tokio::test]
    async fn modes_list_missing_instance_id_is_invalid_params() {
        let v = dispatch("modes/list", json!({})).await;
        assert_eq!(v["code"], -32602);
    }

    #[tokio::test]
    async fn modes_list_unknown_instance_id_is_invalid_params() {
        let v = dispatch(
            "modes/list",
            json!({ "instanceId": "550e8400-e29b-41d4-a716-446655440000" }),
        )
        .await;
        assert_eq!(v["code"], -32602);
    }

    #[tokio::test]
    async fn modes_set_missing_mode_id_is_invalid_params() {
        let v = dispatch(
            "modes/set",
            json!({ "instanceId": "550e8400-e29b-41d4-a716-446655440000" }),
        )
        .await;
        assert_eq!(v["code"], -32602);
    }

    #[tokio::test]
    async fn modes_set_unknown_instance_id_is_invalid_params() {
        let v = dispatch(
            "modes/set",
            json!({ "instanceId": "550e8400-e29b-41d4-a716-446655440000", "modeId": "plan" }),
        )
        .await;
        assert_eq!(v["code"], -32602);
    }

    #[tokio::test]
    async fn modes_unknown_verb_is_method_not_found() {
        let v = dispatch("modes/bogus", Value::Null).await;
        assert_eq!(v["code"], -32601);
    }
}
