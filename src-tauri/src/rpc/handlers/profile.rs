//! `profile/*` namespace — daemon-singleton "currently selected
//! profile" surface. The captain's UI / nvim palette wires its
//! "active profile" pill to this state so cross-frontend selections
//! stay in sync.
//!
//! `profile/get` returns the current value (`None` when no profile
//! is configured AND no client has set one); `profile/set` mutates
//! it after validating against the loaded `[[profiles]]` registry
//! and publishes `acp:profile-changed` so passive consumers refresh
//! without polling.
//!
//! State lives on `AcpAdapter`; this handler is a thin RPC wrapper
//! over the trait methods. Tauri command parity is
//! `tauri/profile_get` + `tauri/profile_set`.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

use crate::rpc::handler::{HandlerCtx, HandlerOutcome, RpcHandler};
use crate::rpc::handlers::util::{map_adapter_err, parse_params};
use crate::rpc::protocol::RpcError;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SetParams {
    profile_id: String,
}

pub struct ProfileHandler;

#[async_trait]
impl RpcHandler for ProfileHandler {
    fn namespace(&self) -> &'static str {
        "profile"
    }

    async fn handle(&self, method: &str, params: Value, ctx: HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
        match method {
            "profile/get" => Ok(HandlerOutcome::Reply(serde_json::json!({
                "profileId": ctx.adapter.selected_profile_id(),
            }))),
            "profile/set" => {
                let SetParams { profile_id } = parse_params::<SetParams>(params, method)?;
                ctx.adapter
                    .set_selected_profile_id(&profile_id)
                    .map(HandlerOutcome::Reply)
                    .map_err(map_adapter_err)
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
    use crate::config::{Config, ProfileConfig};
    use crate::rpc::handler::HandlerCtx;
    use crate::rpc::status::StatusBroadcast;

    fn build_adapter(profiles: Vec<&str>, default: Option<&str>) -> Arc<AcpAdapter> {
        let mut cfg = Config::default();
        for id in profiles {
            cfg.profiles.push(ProfileConfig {
                id: id.into(),
                agent: "claude-code".into(),
                model: None,
                effort: None,
                system_prompt: None,
                mcps: None,
                mcp: None,
                mode: None,
                cwd: None,
                env: std::collections::BTreeMap::new(),
            });
        }
        cfg.profile.default = default.map(str::to_string);
        let shared = Arc::new(RwLock::new(cfg));
        Arc::new(AcpAdapter::with_shared_config(
            shared,
            Arc::new(StatusBroadcast::new(true)),
            Arc::new(DefaultPermissionController::new()) as Arc<dyn PermissionController>,
        ))
    }

    async fn dispatch_with(adapter: Arc<AcpAdapter>, method: &str, params: Value) -> Value {
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
        match ProfileHandler.handle(method, params, ctx).await {
            Ok(HandlerOutcome::Reply(v)) => v,
            Ok(HandlerOutcome::StatusSubscribed(v, _)) => v,
            Ok(HandlerOutcome::EventsSubscribed(v, _, _)) => v,
            Err(err) => json!({ "code": err.code, "message": err.message }),
        }
    }

    #[tokio::test]
    async fn get_returns_config_default_at_boot() {
        let adapter = build_adapter(vec!["strict", "lax"], Some("strict"));
        let v = dispatch_with(adapter, "profile/get", Value::Null).await;
        assert_eq!(v["profileId"], "strict");
    }

    #[tokio::test]
    async fn get_returns_null_when_no_default_configured() {
        let adapter = build_adapter(vec!["strict"], None);
        let v = dispatch_with(adapter, "profile/get", Value::Null).await;
        assert!(v["profileId"].is_null(), "{v}");
    }

    #[tokio::test]
    async fn set_then_get_reflects_new_selection() {
        let adapter = build_adapter(vec!["strict", "lax"], Some("strict"));
        let set = dispatch_with(adapter.clone(), "profile/set", json!({ "profileId": "lax" })).await;
        assert_eq!(set["profileId"], "lax");
        let get = dispatch_with(adapter, "profile/get", Value::Null).await;
        assert_eq!(get["profileId"], "lax");
    }

    #[tokio::test]
    async fn set_unknown_profile_is_invalid_params() {
        let adapter = build_adapter(vec!["strict"], Some("strict"));
        let v = dispatch_with(adapter, "profile/set", json!({ "profileId": "ghost" })).await;
        assert_eq!(v["code"], -32602, "{v}");
    }

    #[tokio::test]
    async fn set_missing_profile_id_is_invalid_params() {
        let adapter = build_adapter(vec!["strict"], Some("strict"));
        let v = dispatch_with(adapter, "profile/set", json!({})).await;
        assert_eq!(v["code"], -32602, "{v}");
    }

    #[tokio::test]
    async fn set_unknown_field_is_invalid_params() {
        let adapter = build_adapter(vec!["strict"], Some("strict"));
        let v = dispatch_with(adapter, "profile/set", json!({ "profileId": "strict", "stray": true })).await;
        assert_eq!(v["code"], -32602, "{v}");
    }

    #[tokio::test]
    async fn unknown_verb_is_method_not_found() {
        let adapter = build_adapter(vec!["strict"], Some("strict"));
        let v = dispatch_with(adapter, "profile/bogus", Value::Null).await;
        assert_eq!(v["code"], -32601, "{v}");
    }
}
