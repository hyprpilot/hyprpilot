use std::path::PathBuf;

use async_trait::async_trait;
use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};

use crate::adapters::{InstanceKey, SpawnSpec};
use crate::rpc::handler::{HandlerCtx, HandlerOutcome, RpcHandler};
use crate::rpc::handlers::util::{map_adapter_err, params_or_default, parse_params};
use crate::rpc::protocol::RpcError;

/// `instances/focus` / `instances/shutdown` / `instances/info` —
/// all take a single UUID string under `id`. Empty-string ids
/// reject at the serde layer with a clean message; malformed
/// uuids reject inside `InstanceKey::parse`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct IdParams {
    #[serde(deserialize_with = "deserialize_non_empty_string")]
    id: String,
}

/// `instances/restart` — takes a UUID id plus an optional `cwd`
/// override. Missing / null `cwd` preserves the resolved agent cwd.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RestartParams {
    #[serde(deserialize_with = "deserialize_non_empty_string")]
    id: String,
    #[serde(default)]
    cwd: Option<PathBuf>,
}

fn deserialize_non_empty_string<'de, D>(de: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::Error;
    let s = String::deserialize(de)?;
    if s.is_empty() {
        return Err(D::Error::custom("instance id cannot be empty"));
    }
    Ok(s)
}

/// `instances/spawn` — every field is optional. Missing profile +
/// agent fall through to the adapter's default-chain, which rejects
/// with `-32602 invalid_params` when nothing resolves.
#[derive(Debug, Deserialize, Default)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct SpawnParams {
    profile_id: Option<String>,
    agent_id: Option<String>,
    cwd: Option<PathBuf>,
    mode: Option<String>,
    model: Option<String>,
}

/// `instances/*` namespace. Registry-level operations on the
/// adapter: list, spawn, focus, restart, shutdown, info. Delegates
/// every method through the `Adapter` trait; param validation +
/// error-mapping only.
pub struct InstancesHandler;

#[async_trait]
impl RpcHandler for InstancesHandler {
    fn namespace(&self) -> &'static str {
        "instances"
    }

    async fn handle(&self, method: &str, params: Value, ctx: HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
        let adapter = ctx
            .adapter
            .as_ref()
            .ok_or_else(|| RpcError::internal_error("adapter not in managed state"))?;

        match method {
            "instances/list" => {
                let items = adapter.list().await;
                let wire: Vec<Value> = items
                    .iter()
                    .map(|i| {
                        json!({
                            "agentId": i.agent_id,
                            "profileId": i.profile_id,
                            "instanceId": i.id,
                            "sessionId": i.session_id,
                            "mode": i.mode,
                        })
                    })
                    .collect();
                Ok(HandlerOutcome::Reply(json!({ "instances": wire })))
            }
            "instances/focus" => {
                let IdParams { id } = parse_params(params, method)?;
                let key = InstanceKey::parse(&id).map_err(map_adapter_err)?;
                let key = adapter.focus(key).await.map_err(map_adapter_err)?;
                Ok(HandlerOutcome::Reply(json!({ "focusedId": key.as_string() })))
            }
            "instances/spawn" => {
                let SpawnParams {
                    profile_id,
                    agent_id,
                    cwd,
                    mode,
                    model,
                } = params_or_default::<SpawnParams>(params, method)?;
                let spec = SpawnSpec {
                    profile_id,
                    agent_id,
                    cwd,
                    mode,
                    model,
                };
                let key = adapter.spawn(spec).await.map_err(map_adapter_err)?;
                Ok(HandlerOutcome::Reply(json!({ "id": key.as_string() })))
            }
            "instances/restart" => {
                let RestartParams { id, cwd } = parse_params(params, method)?;
                let key = InstanceKey::parse(&id).map_err(map_adapter_err)?;
                let key = adapter.restart(key, cwd).await.map_err(map_adapter_err)?;
                Ok(HandlerOutcome::Reply(json!({ "id": key.as_string() })))
            }
            "instances/shutdown" => {
                let IdParams { id } = parse_params(params, method)?;
                let key = InstanceKey::parse(&id).map_err(map_adapter_err)?;
                let key = adapter.shutdown_one(key).await.map_err(map_adapter_err)?;
                Ok(HandlerOutcome::Reply(json!({ "id": key.as_string() })))
            }
            "instances/info" => {
                let IdParams { id } = parse_params(params, method)?;
                let key = InstanceKey::parse(&id).map_err(map_adapter_err)?;
                let info = adapter.info_for(key).await.map_err(map_adapter_err)?;
                Ok(HandlerOutcome::Reply(json!({
                    "agentId": info.agent_id,
                    "profileId": info.profile_id,
                    "instanceId": info.id,
                    "sessionId": info.session_id,
                    "mode": info.mode,
                })))
            }
            other => Err(RpcError::method_not_found(other)),
        }
    }
}
