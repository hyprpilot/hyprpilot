//! `sessions/*` namespace — operations on persisted on-disk session
//! transcripts. Distinct from:
//!   - `session/*` (per-instance ACP wire ops: submit, cancel, info)
//!   - `instances/*` (running agent processes: list, focus, restart)
//!
//! Three verbs:
//!   - `sessions/list` — enumerate the agent's persisted sessions.
//!   - `sessions/forget` — delete a session record on disk. ACP 0.12
//!     has no standardized `session/delete` verb; this stays
//!     `unimplemented!` until upstream lands one.
//!   - `sessions/info` — fetch a single session's projection by id.
//!     `-32602` when the id isn't in the agent's index.

use std::path::PathBuf;

use agent_client_protocol::schema::SessionInfo;
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::rpc::handler::{HandlerCtx, HandlerOutcome, RpcHandler};
use crate::rpc::handlers::util::{params_or_default, parse_params};
use crate::rpc::protocol::RpcError;

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct ListParams {
    instance_id: Option<String>,
    agent_id: Option<String>,
    profile_id: Option<String>,
    cwd: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct IdParams {
    id: String,
}

pub struct SessionsHandler;

#[async_trait]
impl RpcHandler for SessionsHandler {
    fn namespace(&self) -> &'static str {
        "sessions"
    }

    async fn handle(&self, method: &str, params: Value, ctx: HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
        let acp = ctx
            .acp_adapter
            .as_ref()
            .ok_or_else(|| RpcError::internal_error("acp adapter not in managed state"))?;

        match method {
            "sessions/list" => {
                let ListParams {
                    instance_id,
                    agent_id,
                    profile_id,
                    cwd,
                } = params_or_default::<ListParams>(params, method)?;
                let resp = acp
                    .list_sessions(instance_id.as_deref(), agent_id.as_deref(), profile_id.as_deref(), cwd)
                    .await?;
                let sessions: Vec<Value> = resp.sessions.iter().map(project_summary).collect();
                Ok(HandlerOutcome::Reply(json!({ "sessions": sessions })))
            }
            "sessions/forget" => {
                let IdParams { id: _ } = parse_params(params, method)?;
                // ACP 0.12 has no `session/delete` (or equivalent) wire
                // verb — surface the gap loudly per CLAUDE.md "stubs
                // panic" rather than fake-success the call.
                unimplemented!("sessions/forget: ACP 0.12 does not expose a session-delete verb")
            }
            "sessions/info" => {
                let IdParams { id } = parse_params(params, method)?;
                // No ACP `session/get` verb — list + filter is the
                // only available source. List uses the default
                // agent/profile resolution; the resolved ids ride
                // back on the projection so callers can correlate.
                let resp = acp.list_sessions(None, None, None, None).await?;
                let Some(info) = resp.sessions.iter().find(|s| s.session_id.0.as_ref() == id.as_str()) else {
                    return Err(RpcError::invalid_params(format!("no session '{id}'")));
                };
                let (agent_id, profile_id) = default_identity(acp);
                let mut payload = project_summary(info);
                if let Some(obj) = payload.as_object_mut() {
                    obj.insert("agentId".into(), Value::String(agent_id));
                    obj.insert(
                        "profileId".into(),
                        profile_id.map(Value::String).unwrap_or(Value::Null),
                    );
                }
                Ok(HandlerOutcome::Reply(payload))
            }
            other => Err(RpcError::method_not_found(other)),
        }
    }
}

/// Project an ACP `SessionInfo` onto the wire shape — `id`, `title`,
/// `cwd`, `lastTurnAt`, `messageCount`. ACP 0.12 doesn't expose a
/// per-session turn count, so `messageCount` is `null` until upstream
/// lands one.
fn project_summary(info: &SessionInfo) -> Value {
    json!({
        "id": info.session_id.0.as_ref(),
        "title": info.title,
        "cwd": info.cwd.display().to_string(),
        "lastTurnAt": info.updated_at,
        "messageCount": Value::Null,
    })
}

/// Snapshot the default agent + profile ids from config. Used by
/// `sessions/info` to populate the projection with agent identity —
/// `list_sessions(None, None, None)` resolves through the same
/// defaults, so the ids match the source the data was read from.
fn default_identity(acp: &crate::adapters::AcpAdapter) -> (String, Option<String>) {
    let cfg = acp.config.read().expect("AcpAdapter config lock poisoned");
    let agent_id = cfg
        .agents
        .agent
        .default
        .clone()
        .or_else(|| cfg.agents.agents.first().map(|a| a.id.clone()))
        .unwrap_or_default();
    let profile_id = cfg.agents.agent.default_profile.clone();
    (agent_id, profile_id)
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, RwLock};

    use serde_json::json;

    use super::*;
    use crate::adapters::{AcpAdapter, Adapter, DefaultPermissionController, PermissionController};
    use crate::config::Config;
    use crate::rpc::handler::HandlerCtx;
    use crate::rpc::protocol::RequestId;
    use crate::rpc::status::StatusBroadcast;

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
        match SessionsHandler.handle(method, params, ctx).await {
            Ok(HandlerOutcome::Reply(v)) => v,
            Ok(HandlerOutcome::Subscribed(v, _)) => v,
            Err(err) => json!({ "code": err.code, "message": err.message }),
        }
    }

    #[tokio::test]
    async fn list_with_no_default_agent_is_invalid_params() {
        let v = dispatch("sessions/list", Value::Null).await;
        // Default `Config` has no `[[agents]]` — list_sessions resolves
        // through the adapter and bounces with `-32602`.
        assert_eq!(v["code"], -32602, "{v}");
    }

    #[tokio::test]
    async fn list_rejects_unknown_field() {
        let v = dispatch("sessions/list", json!({ "bogus": true })).await;
        assert_eq!(v["code"], -32602);
        let msg = v["message"].as_str().unwrap_or_default();
        assert!(msg.contains("sessions/list params:"), "shape error expected: {v}");
    }

    /// camelCase round-trip: `instanceId` is accepted, snake_case
    /// `instance_id` is rejected as an unknown field.
    #[tokio::test]
    async fn list_camelcase_only() {
        let v = dispatch("sessions/list", json!({ "instance_id": "x" })).await;
        assert_eq!(v["code"], -32602);
        let msg = v["message"].as_str().unwrap_or_default();
        assert!(msg.contains("sessions/list params:"), "{v}");
    }

    #[tokio::test]
    async fn info_missing_id_is_invalid_params() {
        let v = dispatch("sessions/info", Value::Null).await;
        assert_eq!(v["code"], -32602);
    }

    #[tokio::test]
    async fn info_unknown_id_is_invalid_params() {
        // No default agent → `list_sessions` fails first with `-32602`.
        // Either failure is `-32602`; the test pins the contract that
        // unknown ids never escape as `-32603`.
        let v = dispatch("sessions/info", json!({ "id": "ghost" })).await;
        assert_eq!(v["code"], -32602);
    }

    #[tokio::test]
    async fn unknown_verb_is_method_not_found() {
        let v = dispatch("sessions/bogus", Value::Null).await;
        assert_eq!(v["code"], -32601);
    }

    #[tokio::test]
    async fn forget_missing_id_is_invalid_params() {
        // Param-shape error fires before the `unimplemented!()` body.
        let v = dispatch("sessions/forget", Value::Null).await;
        assert_eq!(v["code"], -32602);
    }

    /// `sessions/forget` with a valid id panics today — ACP 0.12 has
    /// no session-delete verb. Locked into a `should_panic` test so
    /// removing the `unimplemented!` without wiring the real path
    /// fails CI.
    #[tokio::test]
    #[should_panic(expected = "ACP 0.12 does not expose a session-delete verb")]
    async fn forget_panics_when_unimplemented() {
        let _ = dispatch("sessions/forget", json!({ "id": "any" })).await;
    }
}
