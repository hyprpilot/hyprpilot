use std::path::PathBuf;

use async_trait::async_trait;
use serde::{Deserialize, Deserializer};
use serde_json::{json, Value};

use crate::adapters::{validate_instance_name, SpawnSpec};
use crate::rpc::handler::{HandlerCtx, HandlerOutcome, RpcHandler};
use crate::rpc::handlers::util::{map_adapter_err, params_or_default};
use crate::rpc::protocol::RpcError;

/// `instances/shutdown` / `instances/info` — `instanceId` accepts
/// UUID OR captain-set name. None falls back to the daemon's
/// focused-instance pointer; an empty string rejects at the serde
/// layer with a clean message.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct InstanceParams {
    #[serde(deserialize_with = "deserialize_optional_non_empty_string")]
    instance_id: Option<String>,
}

/// `instances/focus` — same `instanceId` rules as `InstanceParams`,
/// plus optional auto-spawn behaviour. When `ensure: true` AND the
/// supplied `instanceId` is a slug-shaped name that doesn't resolve
/// to a live instance, the handler spawns one (using the supplied
/// spawn spec), renames it to the slug, then focuses. Mirrors
/// `prompts/send`'s resolve-or-spawn dance so a single keybind can
/// act as "open this named conversation, creating it if needed".
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct FocusParams {
    #[serde(deserialize_with = "deserialize_optional_non_empty_string")]
    instance_id: Option<String>,
    /// When `true` AND `instanceId` is supplied AND `instanceId`
    /// doesn't resolve to a live instance: spawn-and-rename instead
    /// of erroring.
    ensure: bool,
    profile_id: Option<String>,
    agent_id: Option<String>,
    cwd: Option<PathBuf>,
    mode: Option<String>,
    model: Option<String>,
}

/// `instances/restart` — `instanceId` optional (falls back to
/// focused), plus an optional `cwd` override. Missing / null `cwd`
/// preserves the resolved agent cwd.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct RestartParams {
    #[serde(deserialize_with = "deserialize_optional_non_empty_string")]
    instance_id: Option<String>,
    cwd: Option<PathBuf>,
}

/// `instances/rename` — change the addressable name on a live
/// instance. `name == None` clears the name; the slug rule is
/// enforced inside `Adapter::rename`. `instanceId` falls back to
/// focused when omitted (rename-the-current-one ergonomics).
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct RenameParams {
    #[serde(deserialize_with = "deserialize_optional_non_empty_string")]
    instance_id: Option<String>,
    name: Option<String>,
}

fn deserialize_optional_non_empty_string<'de, D>(de: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::Error;
    let opt = Option::<String>::deserialize(de)?;
    match opt {
        Some(s) if s.is_empty() => Err(D::Error::custom("instance id cannot be empty")),
        other => Ok(other),
    }
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
        let adapter = &ctx.adapter;

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
                            "name": i.name,
                            "sessionId": i.session_id,
                            "mode": i.mode,
                        })
                    })
                    .collect();
                Ok(HandlerOutcome::Reply(json!({ "instances": wire })))
            }
            "instances/focus" => {
                let FocusParams {
                    instance_id,
                    ensure,
                    profile_id,
                    agent_id,
                    cwd,
                    mode,
                    model,
                } = params_or_default::<FocusParams>(params, method)?;

                let key = match (instance_id.as_deref(), ensure) {
                    // Ensure-mode with a token: try to resolve, else
                    // spawn-and-rename to that slug. Mirrors
                    // `prompts/send`'s overload.
                    (Some(token), true) => match adapter.resolve_token(token).await {
                        Some(k) => k,
                        None => {
                            let slug = validate_instance_name(token).map_err(|err| {
                                RpcError::invalid_params(format!(
                                    "instance '{token}' not found and not a valid name slug: {err}"
                                ))
                            })?;
                            let spec = SpawnSpec {
                                profile_id,
                                agent_id,
                                cwd,
                                mode,
                                model,
                            };
                            let spawned = adapter.spawn(spec).await.map_err(map_adapter_err)?;
                            adapter.rename(spawned, Some(slug)).await.map_err(map_adapter_err)?;
                            spawned
                        }
                    },
                    _ => resolve_or_focused(adapter.as_ref(), instance_id.as_deref()).await?,
                };
                let key = adapter.focus(key).await.map_err(map_adapter_err)?;
                Ok(HandlerOutcome::Reply(json!({ "instanceId": key.as_string() })))
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
                Ok(HandlerOutcome::Reply(json!({ "instanceId": key.as_string() })))
            }
            "instances/restart" => {
                let RestartParams { instance_id, cwd } = params_or_default::<RestartParams>(params, method)?;
                let key = resolve_or_focused(adapter.as_ref(), instance_id.as_deref()).await?;
                let key = adapter.restart(key, cwd).await.map_err(map_adapter_err)?;
                Ok(HandlerOutcome::Reply(json!({ "instanceId": key.as_string() })))
            }
            "instances/shutdown" => {
                let InstanceParams { instance_id } = params_or_default::<InstanceParams>(params, method)?;
                let key = resolve_or_focused(adapter.as_ref(), instance_id.as_deref()).await?;
                let key = adapter.shutdown_one(key).await.map_err(map_adapter_err)?;
                Ok(HandlerOutcome::Reply(json!({ "instanceId": key.as_string() })))
            }
            "instances/info" => {
                let InstanceParams { instance_id } = params_or_default::<InstanceParams>(params, method)?;
                let key = resolve_or_focused(adapter.as_ref(), instance_id.as_deref()).await?;
                let info = adapter.info_for(key).await.map_err(map_adapter_err)?;
                Ok(HandlerOutcome::Reply(json!({
                    "agentId": info.agent_id,
                    "profileId": info.profile_id,
                    "instanceId": info.id,
                    "name": info.name,
                    "sessionId": info.session_id,
                    "mode": info.mode,
                })))
            }
            "instances/rename" => {
                let RenameParams { instance_id, name } = params_or_default::<RenameParams>(params, method)?;
                let key = resolve_or_focused(adapter.as_ref(), instance_id.as_deref()).await?;
                adapter.rename(key, name.clone()).await.map_err(map_adapter_err)?;
                Ok(HandlerOutcome::Reply(json!({
                    "instanceId": key.as_string(),
                    "name": name,
                })))
            }
            other => Err(RpcError::method_not_found(other)),
        }
    }
}

use super::prompts::resolve_or_focused;
