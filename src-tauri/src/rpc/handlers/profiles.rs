use async_trait::async_trait;
use serde_json::{json, Value};

use crate::rpc::handler::{HandlerCtx, HandlerOutcome, RpcHandler};
use crate::rpc::protocol::RpcError;

/// `profiles/*` namespace — read-only today. Single verb
/// `profiles/list` returns the `[[profiles]]` registry shape the
/// chat-shell picker reads (`id`, `agent`, `model`, `is_default`).
/// Mutating verbs (`set-default`, `active`) were intentionally pared
/// back: config is static at daemon-start, restart-to-change is the
/// model.
pub struct ProfilesHandler;

#[async_trait]
impl RpcHandler for ProfilesHandler {
    fn namespace(&self) -> &'static str {
        "profiles"
    }

    async fn handle(&self, method: &str, _params: Value, ctx: HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
        let adapter = ctx
            .acp_adapter
            .as_ref()
            .ok_or_else(|| RpcError::internal_error("acp adapter not in managed state"))?;

        match method {
            "profiles/list" => Ok(HandlerOutcome::Reply(json!({ "profiles": adapter.list_profiles() }))),
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
    use std::sync::{Arc, RwLock};

    fn fixture() -> (Arc<AcpAdapter>, Arc<RwLock<Config>>) {
        let cfg: Config = toml::from_str(
            r#"
[agent]
default = "claude-code"
default_profile = "ask"

[[agents]]
id = "claude-code"
provider = "acp-claude-code"

[[profiles]]
id = "ask"
agent = "claude-code"

[[profiles]]
id = "strict"
agent = "claude-code"
system_prompt = "be terse"
"#,
        )
        .expect("parses");
        let shared = Arc::new(RwLock::new(cfg));
        let adapter = Arc::new(AcpAdapter::with_shared_config(
            shared.clone(),
            Arc::new(StatusBroadcast::new(true)),
            Arc::new(DefaultPermissionController::new()) as Arc<dyn PermissionController>,
        ));
        (adapter, shared)
    }

    async fn dispatch(method: &str, params: Value) -> Value {
        let (adapter, shared) = fixture();
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
            existing_event_subscription_ids: &[],
            events_tx: None,
        };
        match ProfilesHandler.handle(method, params, ctx).await {
            Ok(HandlerOutcome::Reply(v)) => v,
            Ok(HandlerOutcome::StatusSubscribed(v, _)) => v,
            Ok(HandlerOutcome::EventsSubscribed(v, _)) => v,
            Err(err) => json!({ "code": err.code, "message": err.message }),
        }
    }

    #[tokio::test]
    async fn profiles_list_returns_pared_back_shape() {
        let v = dispatch("profiles/list", Value::Null).await;
        let profiles = v["profiles"].as_array().expect("array");
        assert_eq!(profiles.len(), 2);
        assert_eq!(profiles[0]["id"], "ask");
        assert_eq!(profiles[0]["agent"], "claude-code");
        assert_eq!(profiles[0]["isDefault"], true);
        assert_eq!(profiles[1]["id"], "strict");
        assert_eq!(profiles[1]["isDefault"], false);
        // No `has_prompt`, no `summary`, no `name`.
        assert!(profiles[0].get("has_prompt").is_none());
        assert!(profiles[0].get("summary").is_none());
        assert!(profiles[0].get("name").is_none());
    }

    #[tokio::test]
    async fn profiles_unknown_verb_is_method_not_found() {
        let v = dispatch("profiles/bogus", Value::Null).await;
        assert_eq!(v["code"], -32601);
    }
}
