//! `completion/*` namespace — composer-autocomplete dispatch on the
//! public socket. Mirrors the `tauri/completion_*` proxy entries;
//! both surfaces route through `completion::dispatch::run_*` so
//! detection / cancellation / ranking semantics can't drift between
//! transports.
//!
//! Coverage: `completion/query`, `completion/resolve`,
//! `completion/cancel`, `completion/rank`.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use tauri::Manager;

use crate::completion::dispatch as completion_dispatch;
use crate::completion::source::candidates::CandidateItem;
use crate::completion::{CompletionCancellations, CompletionRegistry};
use crate::rpc::handler::{HandlerCtx, HandlerOutcome, RpcHandler};
use crate::rpc::handlers::util::parse_params;
use crate::rpc::protocol::RpcError;

/// `completion/query` — `text` + `cursor` required; rest optional.
/// `manual` bypasses trigger detection so palette flows can force
/// a fetch even without a leading sigil. `sources` whitelists
/// source ids (e.g. `["path"]` for a cwd palette).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct QueryParams {
    text: String,
    cursor: usize,
    #[serde(default)]
    cwd: Option<PathBuf>,
    #[serde(default)]
    manual: Option<bool>,
    #[serde(default)]
    sources: Option<Vec<String>>,
    /// Currently unused on the daemon side — the registry's `detect`
    /// is instance-agnostic. Carried on the wire so the consumer
    /// doesn't gate on it; future instance-scoped sources land here.
    #[serde(default)]
    #[allow(dead_code)]
    instance_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct ResolveParams {
    resolve_id: String,
    source_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CancelParams {
    request_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RankParams {
    query: String,
    candidates: Vec<CandidateItem>,
}

pub struct CompletionHandler;

#[async_trait]
impl RpcHandler for CompletionHandler {
    fn namespace(&self) -> &'static str {
        "completion"
    }

    async fn handle(&self, method: &str, params: Value, ctx: HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
        match method {
            "completion/query" => {
                // Parse first so missing-required-field surfaces as
                // `-32602 invalid_params` instead of `-32603` from the
                // app-handle probe — the wire-shape error must beat
                // the runtime-state error.
                let p = parse_params::<QueryParams>(params, method)?;
                let app = ctx
                    .app
                    .ok_or_else(|| RpcError::internal_error("completion handler requires a live AppHandle"))?;
                let registry = app
                    .try_state::<Arc<CompletionRegistry>>()
                    .ok_or_else(|| RpcError::internal_error("completion registry state not managed"))?;
                let cancellations = app
                    .try_state::<Arc<CompletionCancellations>>()
                    .ok_or_else(|| RpcError::internal_error("completion cancellations state not managed"))?;
                let value = completion_dispatch::run_query(
                    registry.inner(),
                    cancellations.inner(),
                    &p.text,
                    p.cursor,
                    p.cwd.as_deref(),
                    p.manual.unwrap_or(false),
                    p.sources.as_deref(),
                )
                .await?;
                Ok(HandlerOutcome::Reply(value))
            }
            "completion/resolve" => {
                let p = parse_params::<ResolveParams>(params, method)?;
                let app = ctx
                    .app
                    .ok_or_else(|| RpcError::internal_error("completion handler requires a live AppHandle"))?;
                let registry = app
                    .try_state::<Arc<CompletionRegistry>>()
                    .ok_or_else(|| RpcError::internal_error("completion registry state not managed"))?;
                let value = completion_dispatch::run_resolve(registry.inner(), &p.resolve_id, &p.source_id).await?;
                Ok(HandlerOutcome::Reply(value))
            }
            "completion/cancel" => {
                let p = parse_params::<CancelParams>(params, method)?;
                let app = ctx
                    .app
                    .ok_or_else(|| RpcError::internal_error("completion handler requires a live AppHandle"))?;
                let cancellations = app
                    .try_state::<Arc<CompletionCancellations>>()
                    .ok_or_else(|| RpcError::internal_error("completion cancellations state not managed"))?;
                Ok(HandlerOutcome::Reply(completion_dispatch::run_cancel(
                    cancellations.inner(),
                    &p.request_id,
                )))
            }
            "completion/rank" => {
                let p = parse_params::<RankParams>(params, method)?;
                Ok(HandlerOutcome::Reply(completion_dispatch::run_rank(
                    &p.query,
                    &p.candidates,
                )))
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

    /// Dispatch helper for verbs that don't need a live AppHandle
    /// (rank, plus parse-error paths on the others).
    async fn dispatch_without_app(method: &str, params: Value) -> Value {
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
        match CompletionHandler.handle(method, params, ctx).await {
            Ok(HandlerOutcome::Reply(v)) => v,
            Ok(HandlerOutcome::StatusSubscribed(v, _)) => v,
            Ok(HandlerOutcome::EventsSubscribed(v, _, _)) => v,
            Err(err) => json!({ "code": err.code, "message": err.message }),
        }
    }

    #[tokio::test]
    async fn rank_with_empty_query_returns_identity_order() {
        let v = dispatch_without_app(
            "completion/rank",
            json!({
                "query": "",
                "candidates": [
                    { "id": "alpha", "label": "alpha" },
                    { "id": "beta",  "label": "beta"  },
                ]
            }),
        )
        .await;
        assert_eq!(v["sourceId"], "candidates");
        assert!(v["replacementRange"].is_null());
        let items = v["items"].as_array().expect("items array");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["label"], "alpha");
        assert_eq!(items[1]["label"], "beta");
    }

    #[tokio::test]
    async fn rank_missing_query_is_invalid_params() {
        let v = dispatch_without_app("completion/rank", json!({ "candidates": [] })).await;
        assert_eq!(v["code"], -32602, "{v}");
    }

    /// Query / resolve / cancel all need `ctx.app` to pull state.
    /// With `app: None` the handler returns a typed internal error;
    /// production paths always populate `ctx.app` (Unix socket
    /// `server.rs:147`, WS bridge `ws.rs:417`).
    #[tokio::test]
    async fn query_without_app_is_internal_error() {
        let v = dispatch_without_app("completion/query", json!({ "text": "hi", "cursor": 2 })).await;
        assert_eq!(v["code"], -32603, "{v}");
    }

    #[tokio::test]
    async fn query_missing_required_field_is_invalid_params() {
        let v = dispatch_without_app("completion/query", json!({ "text": "hi" })).await;
        assert_eq!(v["code"], -32602, "{v}");
    }

    #[tokio::test]
    async fn resolve_missing_field_is_invalid_params() {
        let v = dispatch_without_app("completion/resolve", json!({ "resolveId": "x" })).await;
        assert_eq!(v["code"], -32602, "{v}");
    }

    #[tokio::test]
    async fn cancel_missing_field_is_invalid_params() {
        let v = dispatch_without_app("completion/cancel", json!({})).await;
        assert_eq!(v["code"], -32602, "{v}");
    }

    #[tokio::test]
    async fn unknown_verb_is_method_not_found() {
        let v = dispatch_without_app("completion/bogus", Value::Null).await;
        assert_eq!(v["code"], -32601);
    }
}
