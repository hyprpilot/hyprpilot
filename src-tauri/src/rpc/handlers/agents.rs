use async_trait::async_trait;
use serde_json::{json, Value};

use crate::rpc::handler::{HandlerCtx, HandlerOutcome, RpcHandler};
use crate::rpc::protocol::RpcError;

/// `agents/*` namespace. Today only `agents/list` — returns the
/// `[[agents]]` registry shape the chat-shell picker needs (`id`,
/// `binary` = the configured `command`, `kind` = the `AgentProvider`
/// wire name).
pub struct AgentsHandler;

#[async_trait]
impl RpcHandler for AgentsHandler {
    fn namespace(&self) -> &'static str {
        "agents"
    }

    async fn handle(&self, method: &str, _params: Value, ctx: HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
        let adapter = ctx
            .adapter
            .as_ref()
            .ok_or_else(|| RpcError::internal_error("adapter not in managed state"))?;

        match method {
            "agents/list" => {
                // `Adapter::list_agents` emits `{ id, provider, binding, isDefault, capabilities }`
                // — we rename keys on the wire (provider → kind, binding → binary) so
                // downstream pickers don't have to know the internal vocabulary, and
                // pass `capabilities` through verbatim so the UI can gate features
                // per-agent.
                let agents: Vec<Value> = adapter
                    .list_agents()
                    .await
                    .map_err(|err| RpcError::internal_error(err.to_string()))?
                    .into_iter()
                    .map(|v| {
                        json!({
                            "id": v["id"],
                            "binary": v["binding"],
                            "kind": v["provider"],
                            "isDefault": v["isDefault"],
                            "capabilities": v["capabilities"],
                        })
                    })
                    .collect();
                Ok(HandlerOutcome::Reply(json!({ "agents": agents })))
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
    use std::sync::{Arc, RwLock};

    async fn dispatch(method: &str, params: Value) -> Value {
        let cfg: Config = toml::from_str(
            r#"
[agent]
default = "claude-code"

[[agents]]
id = "claude-code"
provider = "acp-claude-code"
command = "bunx"

[[agents]]
id = "codex"
provider = "acp-codex"
"#,
        )
        .expect("parses");
        let shared = Arc::new(RwLock::new(cfg));
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
        match AgentsHandler.handle(method, params, ctx).await {
            Ok(HandlerOutcome::Reply(v)) => v,
            Ok(HandlerOutcome::StatusSubscribed(v, _)) => v,
            Ok(HandlerOutcome::EventsSubscribed(v, _)) => v,
            Err(err) => json!({ "code": err.code, "message": err.message }),
        }
    }

    #[tokio::test]
    async fn agents_list_returns_every_configured_entry() {
        let v = dispatch("agents/list", Value::Null).await;
        let agents = v["agents"].as_array().expect("array");
        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0]["id"], "claude-code");
        assert_eq!(agents[0]["kind"], "acp-claude-code");
        assert_eq!(agents[0]["binary"], "bunx");
        assert_eq!(agents[1]["id"], "codex");
        assert_eq!(agents[1]["kind"], "acp-codex");
        assert!(agents[1]["binary"].is_null());
    }

    #[tokio::test]
    async fn agents_unknown_verb_is_method_not_found() {
        let v = dispatch("agents/bogus", Value::Null).await;
        assert_eq!(v["code"], -32601);
    }
}
