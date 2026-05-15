//! `queue/*` namespace — daemon-side per-instance submit queue.
//!
//! `prompts/send` already auto-routes (busy → enqueue, idle →
//! dispatch); this namespace covers management of the already-queued
//! items: list, edit in place, remove, reorder, clear, and manual
//! dispatch ("send now" — bypasses the queue's auto-route and fires
//! immediately).
//!
//! All verbs accept `instanceId?` and fall back to the focused
//! instance when omitted, matching the convention in
//! `prompts/cancel` + `permissions/respond`.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::adapters::transcript::Attachment;
use crate::rpc::handler::{HandlerCtx, HandlerOutcome, RpcHandler};
use crate::rpc::handlers::util::parse_params;
use crate::rpc::protocol::RpcError;

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct ListParams {
    instance_id: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct EditParams {
    instance_id: Option<String>,
    item_id: String,
    text: String,
    /// Optional. `None` leaves the existing attachments untouched;
    /// `Some(_)` replaces the list wholesale (including `Some(vec![])`
    /// for "clear all attachments on this item").
    attachments: Option<Vec<Attachment>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct RemoveParams {
    instance_id: Option<String>,
    item_id: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct MoveParams {
    instance_id: Option<String>,
    item_id: String,
    position: usize,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct ClearParams {
    instance_id: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct DispatchParams {
    instance_id: Option<String>,
    /// Optional. `None` dispatches the queue head.
    item_id: Option<String>,
}

pub struct QueueHandler;

#[async_trait]
impl RpcHandler for QueueHandler {
    fn namespace(&self) -> &'static str {
        "queue"
    }

    async fn handle(&self, method: &str, params: Value, ctx: HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
        let adapter = &ctx.adapter;

        match method {
            "queue/list" => {
                let p: ListParams = parse_params(params, method)?;
                let items = adapter
                    .queue_list(p.instance_id.as_deref())
                    .await
                    .map_err(crate::rpc::handlers::util::map_adapter_err)?;
                Ok(HandlerOutcome::Reply(json!({ "items": items })))
            }
            "queue/edit" => {
                let p: EditParams = parse_params(params, method)?;
                if p.item_id.is_empty() {
                    return Err(RpcError::invalid_params("queue/edit: itemId must not be empty"));
                }
                let item = adapter
                    .queue_edit(p.instance_id.as_deref(), p.item_id, p.text, p.attachments)
                    .await
                    .map_err(crate::rpc::handlers::util::map_adapter_err)?;
                Ok(HandlerOutcome::Reply(json!({ "item": item })))
            }
            "queue/remove" => {
                let p: RemoveParams = parse_params(params, method)?;
                if p.item_id.is_empty() {
                    return Err(RpcError::invalid_params("queue/remove: itemId must not be empty"));
                }
                let removed = adapter
                    .queue_remove(p.instance_id.as_deref(), p.item_id)
                    .await
                    .map_err(crate::rpc::handlers::util::map_adapter_err)?;
                Ok(HandlerOutcome::Reply(json!({ "removed": removed })))
            }
            "queue/move" => {
                let p: MoveParams = parse_params(params, method)?;
                if p.item_id.is_empty() {
                    return Err(RpcError::invalid_params("queue/move: itemId must not be empty"));
                }
                let moved = adapter
                    .queue_move(p.instance_id.as_deref(), p.item_id, p.position)
                    .await
                    .map_err(crate::rpc::handlers::util::map_adapter_err)?;
                Ok(HandlerOutcome::Reply(json!({ "moved": moved })))
            }
            "queue/clear" => {
                let p: ClearParams = parse_params(params, method)?;
                let cleared = adapter
                    .queue_clear(p.instance_id.as_deref())
                    .await
                    .map_err(crate::rpc::handlers::util::map_adapter_err)?;
                Ok(HandlerOutcome::Reply(json!({ "cleared": cleared })))
            }
            "queue/dispatch" => {
                let p: DispatchParams = parse_params(params, method)?;
                let res = adapter
                    .queue_dispatch(p.instance_id.as_deref(), p.item_id)
                    .await
                    .map_err(crate::rpc::handlers::util::map_adapter_err)?;
                Ok(HandlerOutcome::Reply(serde_json::to_value(res).map_err(|e| {
                    RpcError::internal_error(format!("queue/dispatch: serialise reply: {e}"))
                })?))
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
            shared.clone(),
            Arc::new(StatusBroadcast::new(true)),
            Arc::new(DefaultPermissionController::new()) as Arc<dyn PermissionController>,
        ));
        let status = StatusBroadcast::new(true);
        let dyn_adapter: Arc<dyn Adapter> = adapter.clone();
        let ctx = HandlerCtx {
            app: None,
            status: &status,
            adapter: dyn_adapter,
            config: Some(shared),
            mcps: None,
            already_subscribed: false,
            already_events_subscribed: false,
            started_at: None,
            socket_path: None,
        };
        match QueueHandler.handle(method, params, ctx).await {
            Ok(HandlerOutcome::Reply(v)) => v,
            Ok(HandlerOutcome::StatusSubscribed(v, _)) => v,
            Ok(HandlerOutcome::EventsSubscribed(v, _, _)) => v,
            Err(err) => json!({ "code": err.code, "message": err.message }),
        }
    }

    /// `queue/list` on an empty registry falls back to the focused
    /// instance (which is `None`) and returns an empty list — no
    /// error. Mirrors the `permissions/pending` shape so consumers
    /// can render unconditionally.
    #[tokio::test]
    async fn list_empty_registry_returns_empty_array() {
        let v = dispatch("queue/list", json!({})).await;

        assert_eq!(v["items"], json!([]));
    }

    /// Empty `itemId` is rejected at parse time — the captain typed
    /// a bad arg or the wire shape is broken; we don't no-op.
    #[tokio::test]
    async fn remove_empty_item_id_is_invalid_params() {
        let v = dispatch("queue/remove", json!({ "itemId": "" })).await;

        assert_eq!(v["code"], -32602);
    }

    #[tokio::test]
    async fn edit_empty_item_id_is_invalid_params() {
        let v = dispatch("queue/edit", json!({ "itemId": "", "text": "x" })).await;

        assert_eq!(v["code"], -32602);
    }

    #[tokio::test]
    async fn move_empty_item_id_is_invalid_params() {
        let v = dispatch("queue/move", json!({ "itemId": "", "position": 0 })).await;

        assert_eq!(v["code"], -32602);
    }

    /// Unknown verbs return `-32601 method not found` per JSON-RPC.
    #[tokio::test]
    async fn unknown_verb_is_method_not_found() {
        let v = dispatch("queue/bogus", Value::Null).await;

        assert_eq!(v["code"], -32601);
    }

    /// `queue/clear` against an empty registry is a no-op: 0 cleared.
    /// Same convention as `queue/list` — captain-facing semantics
    /// don't punish a no-op call.
    #[tokio::test]
    async fn clear_empty_registry_reports_zero() {
        let v = dispatch("queue/clear", json!({})).await;

        assert_eq!(v["cleared"], 0);
    }

    /// `deny_unknown_fields` catches typos at the wire boundary.
    #[tokio::test]
    async fn list_rejects_unknown_field() {
        let v = dispatch("queue/list", json!({ "bogus": true })).await;

        assert_eq!(v["code"], -32602);
    }
}
