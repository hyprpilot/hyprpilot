//! `prompts/*` namespace — `prompts/send`, `prompts/cancel`. Mirrors
//! `session/*` semantically with a stricter contract: every call
//! addresses a specific `instance_id` (no auto-spawn fallback).
//! Attachment plumbing is intentionally absent from this surface;
//! palette-picked skills attach via `session/submit` instead.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::adapters::UserTurnInput;
use crate::rpc::handler::{HandlerCtx, HandlerOutcome, RpcHandler};
use crate::rpc::handlers::util::{map_adapter_err, parse_params};
use crate::rpc::protocol::RpcError;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SendParams {
    instance_id: String,
    text: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct CancelParams {
    instance_id: String,
}

pub struct PromptsHandler;

#[async_trait]
impl RpcHandler for PromptsHandler {
    fn namespace(&self) -> &'static str {
        "prompts"
    }

    async fn handle(&self, method: &str, params: Value, ctx: HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
        let adapter = ctx
            .adapter
            .as_ref()
            .ok_or_else(|| RpcError::internal_error("adapter not in managed state"))?;

        match method {
            "prompts/send" => {
                let SendParams { instance_id, text } = parse_params(params, method)?;

                let v = adapter
                    .submit(
                        UserTurnInput::with_attachments(text, Vec::new()),
                        Some(instance_id.as_str()),
                        None,
                        None,
                    )
                    .await
                    .map_err(map_adapter_err)?;

                let accepted = v.get("accepted").and_then(Value::as_bool).unwrap_or(false);
                let session_id = v.get("sessionId").cloned().unwrap_or(Value::Null);
                let resolved_instance_id = v
                    .get("instanceId")
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or(instance_id);

                // Server-assigned turn ids ride a different path (K-281); the
                // existing actor stamps a turn_id internally but it isn't
                // surfaced through the submit reply. Returning null here
                // keeps the wire shape stable; the UI can correlate via
                // `acp:turn-started` events in the meantime.
                Ok(HandlerOutcome::Reply(json!({
                    "accepted": accepted,
                    "instanceId": resolved_instance_id,
                    "turnId": Value::Null,
                    "sessionId": session_id,
                })))
            }
            "prompts/cancel" => {
                let CancelParams { instance_id } = parse_params(params, method)?;
                let v = adapter
                    .cancel(Some(instance_id.as_str()), None)
                    .await
                    .map_err(map_adapter_err)?;
                Ok(HandlerOutcome::Reply(v))
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
            existing_event_subscription_ids: &[],
            events_tx: None,
        };
        match PromptsHandler.handle(method, params, ctx).await {
            Ok(HandlerOutcome::Reply(v)) => v,
            Ok(HandlerOutcome::StatusSubscribed(v, _)) => v,
            Ok(HandlerOutcome::EventsSubscribed(v, _)) => v,
            Err(err) => json!({ "code": err.code, "message": err.message }),
        }
    }

    #[tokio::test]
    async fn send_missing_instance_id_is_invalid_params() {
        let v = dispatch("prompts/send", json!({ "text": "hi" })).await;
        assert_eq!(v["code"], -32602);
    }

    #[tokio::test]
    async fn send_missing_text_is_invalid_params() {
        let v = dispatch("prompts/send", json!({ "instanceId": "x" })).await;
        assert_eq!(v["code"], -32602);
    }

    #[tokio::test]
    async fn cancel_missing_instance_id_is_invalid_params() {
        let v = dispatch("prompts/cancel", json!({})).await;
        assert_eq!(v["code"], -32602);
    }

    #[tokio::test]
    async fn unknown_verb_is_method_not_found() {
        let v = dispatch("prompts/bogus", Value::Null).await;
        assert_eq!(v["code"], -32601);
    }

    /// Unknown wire fields (e.g. a stale client shipping `attachments`)
    /// reject at parse time via `deny_unknown_fields`.
    #[tokio::test]
    async fn send_rejects_unknown_field() {
        let v = dispatch(
            "prompts/send",
            json!({
                "instanceId": "550e8400-e29b-41d4-a716-446655440000",
                "text": "hi",
                "attachments": [],
            }),
        )
        .await;
        assert_eq!(v["code"], -32602);
        let msg = v["message"].as_str().unwrap_or_default();
        assert!(msg.contains("prompts/send params:"), "shape error expected: {v}");
    }
}
