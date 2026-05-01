//! `permissions/*` namespace — `permissions/pending`,
//! `permissions/respond`. Reads from / writes to the same
//! `PermissionController` waiter map the Tauri `permission_reply`
//! command uses; the runtime registers waiters there when
//! `Decision::AskUser` fires.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::adapters::TrustDecision;
use crate::rpc::handler::{HandlerCtx, HandlerOutcome, RpcHandler};
use crate::rpc::handlers::util::{params_or_default, parse_params};
use crate::rpc::protocol::RpcError;

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct PendingParams {
    instance_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct RespondParams {
    request_id: String,
    option_id: String,
    /// Optional trust-store side effect — `"allow"` / `"deny"` writes
    /// `(instance_id, tool)` → decision so the next call from the same
    /// tool short-circuits at decide() lane 1. Absent / null → "once"
    /// semantics, no persistence. UI's "always" buttons set this; the
    /// "once" buttons leave it null.
    #[serde(default)]
    remember: Option<String>,
    #[serde(default)]
    instance_id: Option<String>,
    #[serde(default)]
    tool: Option<String>,
}

pub struct PermissionsHandler;

#[async_trait]
impl RpcHandler for PermissionsHandler {
    fn namespace(&self) -> &'static str {
        "permissions"
    }

    async fn handle(&self, method: &str, params: Value, ctx: HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
        let controller = ctx
            .adapter
            .permissions()
            .ok_or_else(|| RpcError::internal_error("adapter does not broker permissions"))?;

        match method {
            "permissions/pending" => {
                let PendingParams { instance_id } = params_or_default::<PendingParams>(params, method)?;
                let mut snapshots = controller.list_pending().await;
                if let Some(filter) = instance_id.as_deref() {
                    snapshots.retain(|s| s.instance_id.as_deref() == Some(filter));
                }
                Ok(HandlerOutcome::Reply(json!({ "pending": snapshots })))
            }
            "permissions/respond" => {
                let RespondParams {
                    request_id,
                    option_id,
                    remember,
                    instance_id,
                    tool,
                } = parse_params(params, method)?;
                match controller.resolve_if_pending(&request_id, &option_id).await {
                    None => Err(RpcError::invalid_params(format!(
                        "no pending permission for request_id '{request_id}'"
                    ))),
                    Some(false) => Err(RpcError::invalid_params(format!(
                        "option_id '{option_id}' not in permitted set for request_id '{request_id}'"
                    ))),
                    Some(true) => {
                        if let Some(token) = remember.as_deref() {
                            let decision = match token {
                                "allow" => Some(TrustDecision::Allow),
                                "deny" => Some(TrustDecision::Deny),
                                other => {
                                    tracing::warn!(
                                        request_id,
                                        remember = %other,
                                        "permissions/respond: unknown remember token, skipping trust-store update"
                                    );
                                    None
                                }
                            };
                            if let (Some(decision), Some(iid), Some(tname)) =
                                (decision, instance_id.as_ref(), tool.as_ref())
                            {
                                controller.remember(iid, tname, decision).await;
                            } else if decision.is_some() {
                                tracing::warn!(
                                    request_id,
                                    instance_id = ?instance_id,
                                    tool = ?tool,
                                    "permissions/respond: remember requested but instance_id / tool missing"
                                );
                            }
                        }
                        Ok(HandlerOutcome::Reply(json!({ "resolved": true })))
                    }
                }
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
    use crate::adapters::{
        AcpAdapter, Adapter, DefaultPermissionController, PermissionController, PermissionOptionView,
        PermissionOutcome, PermissionRequest, ToolCallRef,
    };
    use crate::config::Config;
    use crate::rpc::handler::HandlerCtx;
    use crate::rpc::protocol::RequestId;
    use crate::rpc::status::StatusBroadcast;

    fn options() -> Vec<PermissionOptionView> {
        vec![
            PermissionOptionView {
                option_id: "allow-once".into(),
                name: "Allow".into(),
                kind: "allow_once".into(),
            },
            PermissionOptionView {
                option_id: "reject-once".into(),
                name: "Reject".into(),
                kind: "reject_once".into(),
            },
        ]
    }

    fn make_request(request_id: &str, instance_id: Option<&str>, tool: &str) -> PermissionRequest {
        PermissionRequest {
            session_id: "sess-1".into(),
            instance_id: instance_id.map(str::to_string),
            request_id: request_id.into(),
            tool_call: ToolCallRef {
                name: tool.into(),
                title: Some(tool.into()),
                raw_args: Some(format!("{tool} args")),
                raw_input: None,
                kind_wire: None,
                content_text: None,
            },
            options: options(),
        }
    }

    async fn dispatch_with_controller(controller: Arc<dyn PermissionController>, method: &str, params: Value) -> Value {
        let shared = Arc::new(RwLock::new(Config::default()));
        let adapter = Arc::new(AcpAdapter::with_shared_config(
            shared.clone(),
            Arc::new(StatusBroadcast::new(true)),
            controller,
        ));
        let status = StatusBroadcast::new(true);
        let id = RequestId::Number(1);
        let dyn_adapter: Arc<dyn Adapter> = adapter.clone();
        let ctx = HandlerCtx {
            app: None,
            status: &status,
            adapter: dyn_adapter,
            config: Some(shared),
            id: &id,
            already_subscribed: false,
            started_at: None,
            socket_path: None,
        };
        match PermissionsHandler.handle(method, params, ctx).await {
            Ok(HandlerOutcome::Reply(v)) => v,
            Ok(HandlerOutcome::StatusSubscribed(v, _)) => v,
            Err(err) => json!({ "code": err.code, "message": err.message }),
        }
    }

    #[tokio::test]
    async fn pending_empty_when_no_waiters() {
        let controller: Arc<dyn PermissionController> = Arc::new(DefaultPermissionController::new());
        let v = dispatch_with_controller(controller, "permissions/pending", Value::Null).await;
        assert_eq!(v["pending"], json!([]));
    }

    #[tokio::test]
    async fn pending_returns_registered_waiter() {
        let inner = Arc::new(DefaultPermissionController::new());
        let _rx = inner
            .register_pending(make_request("req-1", Some("inst-1"), "Bash"))
            .await;
        let controller: Arc<dyn PermissionController> = inner;
        let v = dispatch_with_controller(controller, "permissions/pending", Value::Null).await;
        let pending = v["pending"].as_array().expect("array");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0]["requestId"], "req-1");
        assert_eq!(pending[0]["instanceId"], "inst-1");
        assert_eq!(pending[0]["tool"], "Bash");
        assert!(pending[0]["args"].as_str().unwrap().contains("args"));
        assert_eq!(pending[0]["options"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn pending_filters_by_instance_id() {
        let inner = Arc::new(DefaultPermissionController::new());
        let _rx1 = inner
            .register_pending(make_request("r1", Some("inst-a"), "ToolA"))
            .await;
        let _rx2 = inner
            .register_pending(make_request("r2", Some("inst-b"), "ToolB"))
            .await;
        let controller: Arc<dyn PermissionController> = inner;
        let v = dispatch_with_controller(controller, "permissions/pending", json!({ "instanceId": "inst-a" })).await;
        let pending = v["pending"].as_array().expect("array");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0]["requestId"], "r1");
    }

    #[tokio::test]
    async fn respond_resolves_registered_waiter() {
        let inner = Arc::new(DefaultPermissionController::new());
        let mut rx = inner
            .register_pending(make_request("req-resp", Some("inst-1"), "Bash"))
            .await;
        let controller: Arc<dyn PermissionController> = inner;
        let v = dispatch_with_controller(
            controller,
            "permissions/respond",
            json!({ "requestId": "req-resp", "optionId": "allow-once" }),
        )
        .await;
        assert_eq!(v["resolved"], true);
        let outcome = tokio::time::timeout(std::time::Duration::from_millis(50), &mut rx)
            .await
            .expect("receiver fires")
            .expect("oneshot ok");
        assert_eq!(outcome, PermissionOutcome::Selected("allow-once".into()));
    }

    #[tokio::test]
    async fn respond_unknown_request_is_invalid_params() {
        let controller: Arc<dyn PermissionController> = Arc::new(DefaultPermissionController::new());
        let v = dispatch_with_controller(
            controller,
            "permissions/respond",
            json!({ "requestId": "ghost", "optionId": "allow" }),
        )
        .await;
        assert_eq!(v["code"], -32602);
        let msg = v["message"].as_str().unwrap_or_default();
        assert!(msg.contains("no pending permission"), "{v}");
    }

    /// Known request_id + option_id that isn't in the stored options
    /// list — `-32602` and the waiter must stay registered. We keep a
    /// clone of the inner controller alive past the dispatch call so
    /// the waiter map (and thus the oneshot sender) survives the
    /// temporary `Arc<dyn>` drop inside `dispatch_with_controller`.
    #[tokio::test]
    async fn respond_invalid_option_id_is_invalid_params_and_waiter_intact() {
        let inner = Arc::new(DefaultPermissionController::new());
        let _rx = inner
            .register_pending(make_request("req-bad", Some("inst-1"), "Bash"))
            .await;
        let controller: Arc<dyn PermissionController> = inner.clone();
        let v = dispatch_with_controller(
            controller,
            "permissions/respond",
            json!({ "requestId": "req-bad", "optionId": "ghost-option" }),
        )
        .await;
        assert_eq!(v["code"], -32602);
        let msg = v["message"].as_str().unwrap_or_default();
        assert!(msg.contains("not in permitted set"), "{v}");
        // Waiter map entry intact — `options_for` still returns the stored set.
        assert!(inner.options_for("req-bad").await.is_some());
    }

    #[tokio::test]
    async fn respond_missing_request_id_is_invalid_params() {
        let controller: Arc<dyn PermissionController> = Arc::new(DefaultPermissionController::new());
        let v = dispatch_with_controller(controller, "permissions/respond", json!({ "optionId": "allow" })).await;
        assert_eq!(v["code"], -32602);
    }

    #[tokio::test]
    async fn respond_missing_option_id_is_invalid_params() {
        let controller: Arc<dyn PermissionController> = Arc::new(DefaultPermissionController::new());
        let v = dispatch_with_controller(controller, "permissions/respond", json!({ "requestId": "r1" })).await;
        assert_eq!(v["code"], -32602);
    }

    #[tokio::test]
    async fn unknown_verb_is_method_not_found() {
        let controller: Arc<dyn PermissionController> = Arc::new(DefaultPermissionController::new());
        let v = dispatch_with_controller(controller, "permissions/bogus", Value::Null).await;
        assert_eq!(v["code"], -32601);
    }
}
