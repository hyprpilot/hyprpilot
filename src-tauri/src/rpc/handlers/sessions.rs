//! `sessions/*` namespace — persisted-session catalog reads + resume.
//!
//! Wraps `Adapter::list_sessions` and `Adapter::load_session` so
//! non-SPA clients (nvim plugin, ctl, future remotes) can drive the
//! sessions palette without speaking `tauri/<cmd>`. The desktop
//! SPA keeps its `tauri/session_list` / `tauri/session_load`
//! entries; both surfaces call the same adapter methods.

use std::path::PathBuf;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::rpc::handler::{HandlerCtx, HandlerOutcome, RpcHandler};
use crate::rpc::handlers::util::{map_adapter_err, params_or_default, parse_params};
use crate::rpc::protocol::RpcError;

/// `sessions/list` — every field optional. Filter dimensions match
/// the `Adapter::list_sessions` signature. Empty params returns
/// "every session the addressed agent (or default) can produce."
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct ListParams {
    instance_id: Option<String>,
    agent_id: Option<String>,
    profile_id: Option<String>,
    cwd: Option<PathBuf>,
}

/// `sessions/load` — `sessionId` is the only required field. The
/// daemon adopts it verbatim into the freshly-spawned instance.
/// `cwd` overrides the resolved profile's cwd — agents that scope
/// persisted sessions by cwd (claude-agent-acp) reject loads under
/// a different cwd than the session was created with.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct LoadParams {
    session_id: String,
    instance_id: Option<String>,
    agent_id: Option<String>,
    profile_id: Option<String>,
    cwd: Option<PathBuf>,
}

pub struct SessionsHandler;

#[async_trait]
impl RpcHandler for SessionsHandler {
    fn namespace(&self) -> &'static str {
        "sessions"
    }

    async fn handle(&self, method: &str, params: Value, ctx: HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
        let adapter = &ctx.adapter;
        match method {
            "sessions/list" => {
                let ListParams {
                    instance_id,
                    agent_id,
                    profile_id,
                    cwd,
                } = params_or_default::<ListParams>(params, method)?;
                let value = adapter
                    .list_sessions(instance_id.as_deref(), agent_id.as_deref(), profile_id.as_deref(), cwd)
                    .await
                    .map_err(map_adapter_err)?;
                Ok(HandlerOutcome::Reply(value))
            }
            "sessions/load" => {
                let LoadParams {
                    session_id,
                    instance_id,
                    agent_id,
                    profile_id,
                    cwd,
                } = parse_params::<LoadParams>(params, method)?;
                let key = adapter
                    .load_session(
                        instance_id.as_deref(),
                        agent_id.as_deref(),
                        profile_id.as_deref(),
                        session_id,
                        cwd,
                    )
                    .await
                    .map_err(map_adapter_err)?;
                Ok(HandlerOutcome::Reply(json!({ "instanceId": key.as_string() })))
            }
            other => Err(RpcError::method_not_found(other)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, RwLock};

    use serde_json::json;

    use super::*;
    use crate::adapters::permission::{DefaultPermissionController, PermissionController};
    use crate::adapters::{AcpAdapter, Adapter};
    use crate::config::Config;
    use crate::rpc::handler::HandlerCtx;
    use crate::rpc::status::StatusBroadcast;

    async fn dispatch(method: &str, params: Value) -> Value {
        let shared = Arc::new(RwLock::new(Config::default()));
        let adapter = Arc::new(AcpAdapter::with_shared_config(
            shared,
            Arc::new(StatusBroadcast::new(true)),
            Arc::new(DefaultPermissionController::new()) as Arc<dyn PermissionController>,
        ));
        let status = StatusBroadcast::new(true);
        let dyn_adapter: Arc<dyn Adapter> = adapter;
        let ctx = HandlerCtx {
            app: None,
            status: &status,
            adapter: dyn_adapter,
            config: None,
            mcps: None,
            already_subscribed: false,
            already_events_subscribed: false,
            started_at: None,
            socket_path: None,
        };
        match SessionsHandler.handle(method, params, ctx).await {
            Ok(HandlerOutcome::Reply(v)) => v,
            Ok(HandlerOutcome::StatusSubscribed(v, _)) => v,
            Ok(HandlerOutcome::EventsSubscribed(v, _, _)) => v,
            Err(err) => json!({ "code": err.code, "message": err.message }),
        }
    }

    /// Empty params + an `AcpAdapter` with no configured agents →
    /// the adapter rejects with `AdapterError::InvalidRequest("no
    /// agents configured ...")`, which `map_adapter_err` translates
    /// to `-32602 invalid_params`. Pins the wire shape: the handler
    /// passes through to the adapter and surfaces its error
    /// verbatim, no `-32601` masking when the namespace itself is
    /// wired.
    #[tokio::test]
    async fn list_empty_config_returns_invalid_params() {
        let v = dispatch("sessions/list", Value::Null).await;
        assert_eq!(v["code"], -32602, "{v}");
        assert!(
            v["message"]
                .as_str()
                .unwrap_or_default()
                .contains("no agents configured"),
            "{v}"
        );
    }

    #[tokio::test]
    async fn load_missing_session_id_is_invalid_params() {
        let v = dispatch("sessions/load", json!({})).await;
        assert_eq!(v["code"], -32602, "{v}");
    }

    #[tokio::test]
    async fn load_unknown_field_is_invalid_params() {
        let v = dispatch("sessions/load", json!({ "sessionId": "abc", "stray": true })).await;
        assert_eq!(v["code"], -32602, "{v}");
    }

    #[tokio::test]
    async fn unknown_verb_is_method_not_found() {
        let v = dispatch("sessions/bogus", Value::Null).await;
        assert_eq!(v["code"], -32601);
    }
}
