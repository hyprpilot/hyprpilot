//! `paths/*` namespace — path-resolution helpers exposed to non-SPA
//! clients. Today only `paths/resolve` is wired; future verbs (e.g.
//! `paths/expand_glob`) can land alongside.
//!
//! Mirrors `tauri/paths_resolve` — both surfaces call
//! `tools::path::resolve_absolute` against the daemon's home dir +
//! caller-supplied `cwdBase`.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

use crate::rpc::handler::{HandlerCtx, HandlerOutcome, RpcHandler};
use crate::rpc::handlers::util::parse_params;
use crate::rpc::protocol::RpcError;

/// `paths/resolve` — `~` / `${VAR}` expansion + relative→absolute
/// joining. `cwdBase` is the caller-supplied reference path for
/// resolving relative inputs; absent → resolve against the daemon
/// process cwd.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ResolveParams {
    raw: String,
    #[serde(default)]
    cwd_base: Option<String>,
}

pub struct PathsHandler;

#[async_trait]
impl RpcHandler for PathsHandler {
    fn namespace(&self) -> &'static str {
        "paths"
    }

    async fn handle(&self, method: &str, params: Value, _ctx: HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
        match method {
            "paths/resolve" => {
                let p = parse_params::<ResolveParams>(params, method)?;
                let home = crate::paths::home_dir();
                let home_str = home.to_string_lossy();
                let resolved = crate::tools::path::resolve_absolute(&p.raw, &home_str, p.cwd_base.as_deref());
                Ok(HandlerOutcome::Reply(
                    serde_json::to_value(resolved).unwrap_or(Value::Null),
                ))
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
        match PathsHandler.handle(method, params, ctx).await {
            Ok(HandlerOutcome::Reply(v)) => v,
            Ok(HandlerOutcome::StatusSubscribed(v, _)) => v,
            Ok(HandlerOutcome::EventsSubscribed(v, _, _)) => v,
            Err(err) => json!({ "code": err.code, "message": err.message }),
        }
    }

    #[tokio::test]
    async fn resolve_absolute_path_passes_through() {
        let v = dispatch("paths/resolve", json!({ "raw": "/etc/hosts" })).await;
        assert_eq!(v, json!("/etc/hosts"));
    }

    #[tokio::test]
    async fn resolve_missing_raw_is_invalid_params() {
        let v = dispatch("paths/resolve", json!({})).await;
        assert_eq!(v["code"], -32602);
    }

    #[tokio::test]
    async fn resolve_unknown_field_is_invalid_params() {
        let v = dispatch("paths/resolve", json!({ "raw": "/x", "stray": true })).await;
        assert_eq!(v["code"], -32602);
    }

    #[tokio::test]
    async fn unknown_verb_is_method_not_found() {
        let v = dispatch("paths/bogus", Value::Null).await;
        assert_eq!(v["code"], -32601);
    }
}
