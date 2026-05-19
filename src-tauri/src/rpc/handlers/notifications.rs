//! `notifications/*` namespace — daemon-side "needs attention" tracker.
//!
//! Surface mirrors the Tauri commands of the same name. Frontends
//! (Vue overlay, future nvim plugin, ctl scripting) read the snapshot
//! via `notifications/list` and clear individual entries via
//! `notifications/clear` (backend escape hatch — captains usually
//! resolve via focus / permission answer / prompt send).
//!
//! Live updates ride `events/subscribe` as `acp:notifications-changed`
//! notifications. Identical wire shape to the snapshot, so consumers
//! don't branch on full-vs-delta.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::rpc::handler::{HandlerCtx, HandlerOutcome, RpcHandler};
use crate::rpc::handlers::util::parse_params;
use crate::rpc::protocol::RpcError;

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct ClearParams {
    instance_id: String,
}

pub struct NotificationsHandler;

#[async_trait]
impl RpcHandler for NotificationsHandler {
    fn namespace(&self) -> &'static str {
        "notifications"
    }

    async fn handle(&self, method: &str, params: Value, ctx: HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
        let Some(notifications) = ctx.adapter.notifications() else {
            return Err(RpcError::method_not_found(method));
        };

        match method {
            "notifications/list" => {
                let items = notifications.list_snapshot();
                Ok(HandlerOutcome::Reply(json!({ "items": items })))
            }
            "notifications/clear" => {
                let p: ClearParams = parse_params(params, method)?;
                if p.instance_id.is_empty() {
                    return Err(RpcError::invalid_params(
                        "notifications/clear: instanceId must not be empty",
                    ));
                }
                notifications.clear(&p.instance_id);
                Ok(HandlerOutcome::Reply(json!({ "cleared": true })))
            }
            "notifications/clear_all" => {
                notifications.clear_all();
                Ok(HandlerOutcome::Reply(json!({ "cleared": true })))
            }
            _ => Err(RpcError::method_not_found(method)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::notifications::NotificationReason;
    use crate::adapters::{AcpAdapter, Adapter};
    use crate::config::Config;
    use crate::rpc::status::StatusBroadcast;
    use std::sync::Arc;

    fn ctx_with(adapter: Arc<AcpAdapter>) -> (Arc<AcpAdapter>, Arc<StatusBroadcast>) {
        (adapter, Arc::new(StatusBroadcast::new(true)))
    }

    #[tokio::test]
    async fn list_returns_current_entries() {
        let adapter = Arc::new(AcpAdapter::new(Config::default(), Arc::new(StatusBroadcast::new(true))));
        adapter.notifications().raise("inst-1", NotificationReason::TurnEnded);

        let (a, s) = ctx_with(adapter);
        let ctx = HandlerCtx {
            app: None,
            status: &s,
            adapter: a as Arc<dyn Adapter>,
            config: None,
            mcps: None,
            already_subscribed: false,
            already_events_subscribed: false,
            started_at: None,
            socket_path: None,
        };

        let outcome = NotificationsHandler
            .handle("notifications/list", Value::Null, ctx)
            .await
            .expect("ok");
        let HandlerOutcome::Reply(v) = outcome else {
            panic!("expected Reply");
        };
        let items = v["items"].as_array().expect("items array");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["instanceId"], "inst-1");
    }

    #[tokio::test]
    async fn clear_drops_entry() {
        let adapter = Arc::new(AcpAdapter::new(Config::default(), Arc::new(StatusBroadcast::new(true))));
        adapter.notifications().raise("inst-1", NotificationReason::TurnEnded);

        let (a, s) = ctx_with(adapter.clone());
        let ctx = HandlerCtx {
            app: None,
            status: &s,
            adapter: a as Arc<dyn Adapter>,
            config: None,
            mcps: None,
            already_subscribed: false,
            already_events_subscribed: false,
            started_at: None,
            socket_path: None,
        };

        let outcome = NotificationsHandler
            .handle("notifications/clear", json!({ "instanceId": "inst-1" }), ctx)
            .await
            .expect("ok");
        let HandlerOutcome::Reply(v) = outcome else {
            panic!("expected Reply");
        };
        assert_eq!(v["cleared"], true);
        assert!(adapter.notifications().list_snapshot().is_empty());
    }

    #[tokio::test]
    async fn clear_rejects_empty_instance_id() {
        let adapter = Arc::new(AcpAdapter::new(Config::default(), Arc::new(StatusBroadcast::new(true))));
        let (a, s) = ctx_with(adapter);
        let ctx = HandlerCtx {
            app: None,
            status: &s,
            adapter: a as Arc<dyn Adapter>,
            config: None,
            mcps: None,
            already_subscribed: false,
            already_events_subscribed: false,
            started_at: None,
            socket_path: None,
        };

        let result = NotificationsHandler
            .handle("notifications/clear", json!({ "instanceId": "" }), ctx)
            .await;
        let err = match result {
            Ok(_) => panic!("expected reject on empty instanceId"),
            Err(e) => e,
        };
        assert_eq!(err.code, RpcError::CODE_INVALID_PARAMS);
    }
}
