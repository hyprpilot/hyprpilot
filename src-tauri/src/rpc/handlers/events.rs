//! `events/*` namespace — `events/subscribe`. Streams live
//! `InstanceEvent`s as JSON-RPC notifications (`events/changed`)
//! over the unix socket. This is the local-IPC mirror of the WS
//! remote bridge's event broadcast — same broadcast source
//! (`Adapter::subscribe()`), same event names (
//! `InstanceEvent::event_name()`), same payload shape.
//!
//! Powering: a Neovim plugin (or `ctl events --watch`, or any other
//! second frontend) connects to the unix socket, calls
//! `events/subscribe`, and receives a stream of notifications without
//! polling `instance/snapshot/chat`. Replies, prompts, permission
//! responses, and other writes go through the existing JSON-RPC
//! verbs (`prompts/send`, `permissions/respond`, `tauri/*`) on the
//! same connection.
//!
//! Filter: optional `{instance_id: string}` param scopes the stream
//! to events tagged with that id. Daemon-global events
//! (`daemon:reloaded`, `acp:instances-changed`, `acp:instances-focused`)
//! pass through unconditionally — a buffer pinned to one instance
//! still wants to know when the instance set or focus changes.
//!
//! Single subscription per connection. A second `events/subscribe`
//! on the same socket returns `-32600 invalid request` (mirrors the
//! `status/subscribe` rule). To change the filter, reconnect.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::rpc::handler::{EventsFilter, HandlerCtx, HandlerOutcome, RpcHandler};
use crate::rpc::protocol::RpcError;

pub struct EventsHandler;

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct SubscribeParams {
    /// When set, only events tagged with this instance id are
    /// emitted to this connection. Daemon-global events still pass.
    instance_id: Option<String>,
}

#[async_trait]
impl RpcHandler for EventsHandler {
    fn namespace(&self) -> &'static str {
        "events"
    }

    async fn handle(&self, method: &str, params: Value, ctx: HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
        match method {
            "events/subscribe" => {
                if ctx.already_events_subscribed {
                    return Err(RpcError::invalid_request(
                        "connection already subscribed to events/changed",
                    ));
                }
                let parsed: SubscribeParams = if params.is_null() {
                    SubscribeParams::default()
                } else {
                    serde_json::from_value(params).map_err(|err| RpcError::invalid_params(err.to_string()))?
                };
                let filter = EventsFilter {
                    instance_id: parsed.instance_id.filter(|s| !s.is_empty()),
                };
                let rx = ctx.adapter.subscribe();
                // Ack value carries the filter back so the captain
                // can `console.log(reply.filter)` and verify what
                // the server actually applied.
                let ack = json!({
                    "subscribed": true,
                    "filter": {
                        "instanceId": filter.instance_id,
                    },
                });
                Ok(HandlerOutcome::EventsSubscribed(ack, Box::new(rx), filter))
            }
            other => Err(RpcError::method_not_found(other)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::{AcpAdapter, Adapter};
    use crate::config::Config;
    use crate::rpc::status::StatusBroadcast;
    use std::sync::Arc;

    fn fixture(already_events_subscribed: bool) -> (Arc<dyn Adapter>, StatusBroadcast) {
        let acp = Arc::new(AcpAdapter::new(Config::default(), Arc::new(StatusBroadcast::new(true))));
        let status = StatusBroadcast::new(false);
        let _ = already_events_subscribed; // ctx is built per-call below
        (acp, status)
    }

    fn ctx<'a>(
        adapter: &Arc<dyn Adapter>,
        status: &'a StatusBroadcast,
        already_events_subscribed: bool,
    ) -> HandlerCtx<'a> {
        HandlerCtx {
            app: None,
            status,
            adapter: adapter.clone(),
            config: None,
            mcps: None,
            already_subscribed: false,
            already_events_subscribed,
            started_at: None,
            socket_path: None,
        }
    }

    /// Subscribe with an instance filter — the ack carries it back so
    /// the peer can verify what the server actually applied.
    #[tokio::test]
    async fn subscribe_returns_filter_in_ack() {
        let (adapter, status) = fixture(false);
        let outcome = EventsHandler
            .handle(
                "events/subscribe",
                json!({"instanceId": "abc-123"}),
                ctx(&adapter, &status, false),
            )
            .await;
        match outcome {
            Ok(HandlerOutcome::EventsSubscribed(ack, _rx, filter)) => {
                assert_eq!(filter.instance_id.as_deref(), Some("abc-123"));
                assert_eq!(ack["subscribed"], json!(true));
                assert_eq!(ack["filter"]["instanceId"], json!("abc-123"));
            }
            _ => panic!("expected EventsSubscribed"),
        }
    }

    #[tokio::test]
    async fn second_subscribe_on_same_connection_rejects() {
        let (adapter, status) = fixture(false);
        let res = EventsHandler
            .handle("events/subscribe", Value::Null, ctx(&adapter, &status, true))
            .await;
        match res {
            Err(e) => assert_eq!(e.code, -32600),
            Ok(_) => panic!("second subscribe must be rejected"),
        }
    }

    #[tokio::test]
    async fn empty_string_instance_id_treated_as_no_filter() {
        let (adapter, status) = fixture(false);
        let outcome = EventsHandler
            .handle(
                "events/subscribe",
                json!({"instanceId": ""}),
                ctx(&adapter, &status, false),
            )
            .await;
        match outcome {
            Ok(HandlerOutcome::EventsSubscribed(_, _rx, filter)) => {
                assert!(filter.instance_id.is_none(), "empty string should drop the filter");
            }
            _ => panic!("expected EventsSubscribed"),
        }
    }

    #[tokio::test]
    async fn null_params_subscribes_with_no_filter() {
        let (adapter, status) = fixture(false);
        let outcome = EventsHandler
            .handle("events/subscribe", Value::Null, ctx(&adapter, &status, false))
            .await;
        match outcome {
            Ok(HandlerOutcome::EventsSubscribed(_, _, filter)) => assert!(filter.instance_id.is_none()),
            _ => panic!("expected EventsSubscribed"),
        }
    }

    #[tokio::test]
    async fn unknown_method_in_namespace_returns_method_not_found() {
        let (adapter, status) = fixture(false);
        let res = EventsHandler
            .handle("events/unknown", Value::Null, ctx(&adapter, &status, false))
            .await;
        match res {
            Err(e) => assert_eq!(e.code, -32601),
            Ok(_) => panic!("unknown method must error"),
        }
    }
}
