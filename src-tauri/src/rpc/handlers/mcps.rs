use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::adapters::InstanceKey;
use crate::mcp::MCPsRegistry;
use crate::rpc::handler::{HandlerCtx, HandlerOutcome, RpcHandler};
use crate::rpc::handlers::util::{params_or_default, parse_params};
use crate::rpc::protocol::RpcError;

/// `mcps/*` namespace — exposes the global `[[mcps]]` catalog and the
/// per-instance enabled-list overrides on the ACP adapter. Two verbs
/// today: `mcps/list` (catalog with `enabled` flag), `mcps/set`
/// (install override + restart the addressed instance).
pub struct MCPsHandler {
    registry: Arc<MCPsRegistry>,
}

impl MCPsHandler {
    #[must_use]
    pub fn new(registry: Arc<MCPsRegistry>) -> Self {
        Self { registry }
    }
}

#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct ListParams {
    instance_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SetParams {
    instance_id: String,
    enabled: Vec<String>,
}

#[async_trait]
impl RpcHandler for MCPsHandler {
    fn namespace(&self) -> &'static str {
        "mcps"
    }

    async fn handle(&self, method: &str, params: Value, ctx: HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
        match method {
            "mcps/list" => {
                let ListParams { instance_id } = params_or_default::<ListParams>(params, method)?;
                let catalog = self.registry.list();
                let enabled_filter = match instance_id {
                    Some(id) => {
                        let acp_adapter = ctx
                            .acp_adapter
                            .as_ref()
                            .ok_or_else(|| RpcError::internal_error("acp adapter not in managed state"))?;
                        let key = InstanceKey::parse(&id)
                            .map_err(|err| RpcError::invalid_params(format!("mcps/list instance_id '{id}': {err}")))?;
                        // None → all-enabled (no profile filter); Some(list) → only those names.
                        Some(acp_adapter.effective_mcps_for(key).await)
                    }
                    None => None,
                };
                let items: Vec<Value> = catalog
                    .iter()
                    .map(|m| {
                        let enabled = match &enabled_filter {
                            None => true,
                            Some(None) => true,
                            Some(Some(list)) => list.iter().any(|n| n == &m.name),
                        };
                        json!({
                            "name": m.name,
                            "command": m.command,
                            "enabled": enabled,
                        })
                    })
                    .collect();
                Ok(HandlerOutcome::Reply(json!({ "mcps": items })))
            }
            "mcps/set" => {
                let SetParams { instance_id, enabled } = parse_params(params, method)?;
                let acp_adapter = ctx
                    .acp_adapter
                    .as_ref()
                    .ok_or_else(|| RpcError::internal_error("acp adapter not in managed state"))?;
                let key = InstanceKey::parse(&instance_id)
                    .map_err(|err| RpcError::invalid_params(format!("mcps/set instance_id '{instance_id}': {err}")))?;
                let _ = acp_adapter.contains_instance(&instance_id).await?;

                // Validate every requested name resolves to a catalog entry.
                let catalog = self.registry.list();
                for name in &enabled {
                    if !catalog.iter().any(|m| &m.name == name) {
                        return Err(RpcError::invalid_params(format!(
                            "mcps/set: '{name}' not in [[mcps]] catalog"
                        )));
                    }
                }

                acp_adapter.set_mcps_override(key, enabled);
                acp_adapter.restart_instance(key, None).await?;
                Ok(HandlerOutcome::Reply(json!({ "restarted": true })))
            }
            other => Err(RpcError::method_not_found(other)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::RwLock;

    use super::*;
    use crate::adapters::{AcpAdapter, Adapter, DefaultPermissionController, PermissionController};
    use crate::config::Config;
    use crate::mcp::{MCPDefinition, MCPsRegistry};
    use crate::rpc::protocol::RequestId;
    use crate::rpc::status::StatusBroadcast;

    fn def(name: &str, cmd: &str) -> MCPDefinition {
        MCPDefinition {
            name: name.to_string(),
            command: cmd.to_string(),
            args: Vec::new(),
            env: Default::default(),
            scope: None,
        }
    }

    fn build_handler(defs: Vec<MCPDefinition>) -> MCPsHandler {
        MCPsHandler::new(Arc::new(MCPsRegistry::new(defs)))
    }

    fn build_adapter(cfg: Config) -> Arc<AcpAdapter> {
        let shared = Arc::new(RwLock::new(cfg));
        Arc::new(AcpAdapter::with_shared_config(
            shared,
            Arc::new(StatusBroadcast::new(true)),
            Arc::new(DefaultPermissionController::new()) as Arc<dyn PermissionController>,
        ))
    }

    async fn run(handler: &MCPsHandler, adapter: Arc<AcpAdapter>, method: &str, params: Value) -> Value {
        let status = StatusBroadcast::new(true);
        let id = RequestId::Number(1);
        let dyn_adapter: Arc<dyn Adapter> = adapter.clone();
        let config = adapter.shared_config();
        let ctx = HandlerCtx {
            app: None,
            status: &status,
            adapter: Some(dyn_adapter),
            acp_adapter: Some(adapter),
            config: Some(config),
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
        match handler.handle(method, params, ctx).await {
            Ok(HandlerOutcome::Reply(v)) => v,
            Ok(HandlerOutcome::StatusSubscribed(v, _)) => v,
            Ok(HandlerOutcome::EventsSubscribed(v, _)) => v,
            Err(err) => json!({ "code": err.code, "message": err.message }),
        }
    }

    #[tokio::test]
    async fn list_without_instance_returns_full_catalog_all_enabled() {
        let handler = build_handler(vec![def("fs", "uvx"), def("rg", "rg")]);
        let adapter = build_adapter(Config::default());
        let v = run(&handler, adapter, "mcps/list", Value::Null).await;
        let mcps = v["mcps"].as_array().expect("array");
        assert_eq!(mcps.len(), 2);
        assert_eq!(mcps[0]["name"], "fs");
        assert_eq!(mcps[0]["enabled"], true);
        assert_eq!(mcps[1]["name"], "rg");
        assert_eq!(mcps[1]["enabled"], true);
    }

    #[tokio::test]
    async fn list_unknown_instance_id_is_invalid_params() {
        let handler = build_handler(vec![def("fs", "uvx")]);
        let adapter = build_adapter(Config::default());
        let v = run(&handler, adapter, "mcps/list", json!({ "instanceId": "not-a-uuid" })).await;
        assert_eq!(v["code"], -32602);
    }

    #[tokio::test]
    async fn set_unknown_instance_id_is_invalid_params() {
        let handler = build_handler(vec![def("fs", "uvx")]);
        let adapter = build_adapter(Config::default());
        let v = run(
            &handler,
            adapter,
            "mcps/set",
            json!({
                "instanceId": "550e8400-e29b-41d4-a716-446655440000",
                "enabled": ["fs"],
            }),
        )
        .await;
        // Unknown instance id (not registered) returns invalid_params via contains_instance.
        assert_eq!(v["code"], -32602);
    }

    /// Spawn a registered instance against a dead-child config. The
    /// actor enters Error state asynchronously, but the registry
    /// records the key + handle synchronously, so subsequent
    /// `contains_instance` lookups + `restart_instance` calls find it.
    async fn dead_child_adapter_with_registered_key() -> (Arc<AcpAdapter>, InstanceKey) {
        let cfg: Config = toml::from_str(
            r#"
[agent]
default = "dead"

[[agents]]
id = "dead"
provider = "acp-claude-code"
command = "/bin/false"
"#,
        )
        .expect("parses");
        let adapter = build_adapter(cfg);
        let key = adapter
            .spawn_instance(crate::adapters::SpawnSpec::default())
            .await
            .expect("spawn ok");
        (adapter, key)
    }

    #[tokio::test]
    async fn set_unknown_mcp_name_is_invalid_params() {
        let (adapter, key) = dead_child_adapter_with_registered_key().await;
        let handler = build_handler(vec![def("fs", "uvx")]);
        let v = run(
            &handler,
            adapter,
            "mcps/set",
            json!({
                "instanceId": key.as_string(),
                "enabled": ["bogus"],
            }),
        )
        .await;
        assert_eq!(v["code"], -32602);
        let msg = v["message"].as_str().unwrap_or_default();
        assert!(msg.contains("'bogus'") && msg.contains("catalog"), "{v}");
    }

    #[tokio::test]
    async fn unknown_verb_is_method_not_found() {
        let handler = build_handler(vec![def("fs", "uvx")]);
        let adapter = build_adapter(Config::default());
        let v = run(&handler, adapter, "mcps/bogus", Value::Null).await;
        assert_eq!(v["code"], -32601);
    }

    #[tokio::test]
    async fn set_records_override_and_restarts_instance() {
        let (adapter, key) = dead_child_adapter_with_registered_key().await;
        let handler = build_handler(vec![def("fs", "uvx"), def("rg", "rg")]);

        let v = run(
            &handler,
            adapter.clone(),
            "mcps/set",
            json!({
                "instanceId": key.as_string(),
                "enabled": ["fs"],
            }),
        )
        .await;
        assert_eq!(v["restarted"], true);

        // The override survives the restart — list should now show
        // `fs` enabled and `rg` disabled.
        let v = run(&handler, adapter, "mcps/list", json!({ "instanceId": key.as_string() })).await;
        let mcps = v["mcps"].as_array().expect("array");
        let fs = mcps.iter().find(|m| m["name"] == "fs").unwrap();
        let rg = mcps.iter().find(|m| m["name"] == "rg").unwrap();
        assert_eq!(fs["enabled"], true);
        assert_eq!(rg["enabled"], false);
    }
}
