//! `events/*` namespace — connection-scoped fan-out of every event the
//! generic `AdapterRegistry` broadcasts.
//!
//! Subscribe spawns a per-subscription filter task that consumes the
//! adapter's broadcast, applies `(topics?, instanceId?)` scoping, and
//! pushes onto the connection's shared mpsc the server task drains as
//! `events/notify` notifications. Each subscription holds a oneshot
//! cancel sender — dropping it (on `events/unsubscribe` or connection
//! close) terminates the filter task.
//!
//! Bounded mpsc per CLAUDE.md "drop on slow-consumer backpressure" —
//! `try_send` Full → `warn!` + drop. Broadcast Lagged → log + continue.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::sync::{broadcast, mpsc, oneshot};
use tracing::warn;
use uuid::Uuid;

use crate::adapters::{AcpAdapter, InstanceEvent};
#[cfg(test)]
use crate::rpc::handler::EventsConnectionTx;
use crate::rpc::handler::{EventsSubscription, HandlerCtx, HandlerOutcome, RpcHandler};
use crate::rpc::handlers::util::params_or_default;
use crate::rpc::protocol::{EventsNotifyParams, RpcError};
use crate::rpc::topic::{event_instance_id, WireTopic};

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
struct SubscribeParams {
    /// Empty / `None` = firehose every topic.
    topics: Option<Vec<WireTopic>>,
    /// Empty / `None` = events from every instance.
    instance_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct UnsubscribeParams {
    subscription_id: String,
}

pub struct EventsHandler;

#[async_trait]
impl RpcHandler for EventsHandler {
    fn namespace(&self) -> &'static str {
        "events"
    }

    async fn handle(&self, method: &str, params: Value, ctx: HandlerCtx<'_>) -> Result<HandlerOutcome, RpcError> {
        match method {
            "events/subscribe" => {
                let SubscribeParams { topics, instance_id } = params_or_default(params, method)?;
                let acp = ctx
                    .acp_adapter
                    .as_ref()
                    .ok_or_else(|| RpcError::internal_error("acp adapter not in managed state"))?;
                let conn_tx = ctx
                    .events_tx
                    .ok_or_else(|| RpcError::internal_error("events outbound channel not initialised"))?;

                let topic_filter: HashSet<WireTopic> = topics.unwrap_or_default().into_iter().collect();
                for t in &topic_filter {
                    if t.is_unwired() {
                        warn!(topic = ?t, "events/subscribe: topic accepted but no producer wires events to it yet");
                    }
                }

                let subscription_id = Uuid::new_v4().to_string();
                let (cancel_tx, cancel_rx) = oneshot::channel::<()>();

                spawn_filter_task(
                    acp.clone(),
                    subscription_id.clone(),
                    topic_filter,
                    instance_id,
                    conn_tx.tx.clone(),
                    cancel_rx,
                );

                Ok(HandlerOutcome::EventsSubscribed(
                    json!({ "subscriptionId": subscription_id }),
                    EventsSubscription {
                        subscription_id,
                        cancel: cancel_tx,
                    },
                ))
            }
            "events/unsubscribe" => {
                let UnsubscribeParams { subscription_id } = crate::rpc::handlers::util::parse_params(params, method)?;
                // Eviction happens in the connection loop: it inspects
                // the response payload (`{ unsubscribed: true,
                // subscriptionId }`) and removes the matching entry
                // from its vec, dropping the cancel sender, which
                // terminates the filter task. Unknown ids return
                // `false` instead of an error — same shape, no
                // eviction triggered.
                let exists = ctx
                    .existing_event_subscription_ids
                    .iter()
                    .any(|s| s == &subscription_id);
                if exists {
                    Ok(HandlerOutcome::Reply(json!({
                        "unsubscribed": true,
                        "subscriptionId": subscription_id,
                    })))
                } else {
                    Ok(HandlerOutcome::Reply(json!({
                        "unsubscribed": false,
                        "subscriptionId": subscription_id,
                    })))
                }
            }
            other => Err(RpcError::method_not_found(other)),
        }
    }
}

/// Per-subscription forwarder. Owns a broadcast receiver + the
/// connection's shared mpsc sender. Exits on cancel signal (drop of
/// the matching `EventsSubscription`), broadcast Closed, or on a
/// `try_send` to a closed mpsc (connection vanished).
fn spawn_filter_task(
    acp: Arc<AcpAdapter>,
    subscription_id: String,
    topic_filter: HashSet<WireTopic>,
    instance_filter: Option<String>,
    tx: mpsc::Sender<EventsNotifyParams>,
    mut cancel: oneshot::Receiver<()>,
) {
    let mut rx = acp.subscribe_events();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                _ = &mut cancel => return,
                next = rx.recv() => {
                    match next {
                        Ok(evt) => {
                            if !accept(&evt, &topic_filter, instance_filter.as_deref()) {
                                continue;
                            }
                            let Some(params) = build_notify_params(&subscription_id, &evt) else {
                                continue;
                            };
                            match tx.try_send(params) {
                                Ok(()) => {}
                                Err(mpsc::error::TrySendError::Full(_)) => {
                                    warn!(
                                        subscription_id = %subscription_id,
                                        "events: dropping notification, slow consumer"
                                    );
                                }
                                Err(mpsc::error::TrySendError::Closed(_)) => return,
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            warn!(subscription_id = %subscription_id, n, "events: subscriber lagged");
                            continue;
                        }
                        Err(broadcast::error::RecvError::Closed) => return,
                    }
                }
            }
        }
    });
}

fn accept(evt: &InstanceEvent, topics: &HashSet<WireTopic>, instance_filter: Option<&str>) -> bool {
    if !topics.is_empty() && !topics.iter().any(|t| t.matches(evt)) {
        return false;
    }
    if let Some(want) = instance_filter {
        match event_instance_id(evt) {
            Some(got) if got == want => true,
            // Registry-wide events (`InstancesChanged`/`InstancesFocused`)
            // carry no instance id — scoped out under an explicit filter.
            _ => false,
        }
    } else {
        true
    }
}

/// Project an `InstanceEvent` into the `events/notify` params shape.
/// Topic field always reflects one of the canonical (non-alias)
/// `WireTopic` variants — consumers that subscribed to an alias
/// (`state.changed`, `transcript.chunk`, `permission.requested`) still
/// see `instance.state` / `instance.transcript` /
/// `instance.permission_request` on the wire.
fn build_notify_params(subscription_id: &str, evt: &InstanceEvent) -> Option<EventsNotifyParams> {
    let topic = wire_topic_for(evt)?;
    let payload = serde_json::to_value(evt).ok()?;
    Some(EventsNotifyParams {
        subscription_id: subscription_id.to_string(),
        topic,
        instance_id: event_instance_id(evt).map(str::to_string),
        payload,
    })
}

fn wire_topic_for(evt: &InstanceEvent) -> Option<WireTopic> {
    Some(match evt {
        InstanceEvent::State { .. } => WireTopic::InstanceState,
        InstanceEvent::Transcript { .. } => WireTopic::InstanceTranscript,
        InstanceEvent::PermissionRequest { .. } => WireTopic::InstancePermissionRequest,
        InstanceEvent::TurnStarted { .. } => WireTopic::InstanceTurnStarted,
        InstanceEvent::TurnEnded { .. } => WireTopic::InstanceTurnEnded,
        InstanceEvent::InstancesChanged { .. } => WireTopic::InstancesChanged,
        InstanceEvent::InstancesFocused { .. } => WireTopic::InstancesFocused,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::InstanceState;
    use crate::config::Config;
    use crate::rpc::protocol::RequestId;
    use crate::rpc::status::StatusBroadcast;
    use serde_json::json;

    fn ctx_with<'a>(
        status: &'a StatusBroadcast,
        id: &'a RequestId,
        acp: Arc<AcpAdapter>,
        existing: &'a [String],
        events_tx: Option<&'a EventsConnectionTx>,
    ) -> HandlerCtx<'a> {
        HandlerCtx {
            app: None,
            status,
            adapter: Some(acp.clone()),
            acp_adapter: Some(acp),
            config: None,
            id,
            already_subscribed: false,
            existing_event_subscription_ids: existing,
            events_tx,
        }
    }

    fn fresh_events_tx() -> (EventsConnectionTx, mpsc::Receiver<EventsNotifyParams>) {
        let (tx, rx) = mpsc::channel::<EventsNotifyParams>(8);
        (EventsConnectionTx { tx }, rx)
    }

    #[tokio::test]
    async fn subscribe_returns_subscription_id_and_handle() {
        let acp = Arc::new(AcpAdapter::new(Config::default(), Arc::new(StatusBroadcast::new(true))));
        let status = StatusBroadcast::new(true);
        let id = RequestId::Number(1);
        let (events_tx, _events_rx) = fresh_events_tx();
        let ctx = ctx_with(&status, &id, acp, &[], Some(&events_tx));

        let outcome = EventsHandler
            .handle("events/subscribe", Value::Null, ctx)
            .await
            .unwrap();
        match outcome {
            HandlerOutcome::EventsSubscribed(reply, handle) => {
                let sid = reply["subscriptionId"].as_str().unwrap();
                assert!(!sid.is_empty());
                assert_eq!(handle.subscription_id, sid);
            }
            _ => panic!("expected EventsSubscribed"),
        }
    }

    #[tokio::test]
    async fn subscribe_with_explicit_topics_round_trips() {
        let acp = Arc::new(AcpAdapter::new(Config::default(), Arc::new(StatusBroadcast::new(true))));
        let status = StatusBroadcast::new(true);
        let id = RequestId::Number(1);
        let (events_tx, _events_rx) = fresh_events_tx();
        let ctx = ctx_with(&status, &id, acp, &[], Some(&events_tx));

        let outcome = EventsHandler
            .handle(
                "events/subscribe",
                json!({ "topics": ["instances.changed", "instance.state"] }),
                ctx,
            )
            .await
            .unwrap();
        assert!(matches!(outcome, HandlerOutcome::EventsSubscribed(..)));
    }

    #[tokio::test]
    async fn subscribe_rejects_unknown_topic() {
        let acp = Arc::new(AcpAdapter::new(Config::default(), Arc::new(StatusBroadcast::new(true))));
        let status = StatusBroadcast::new(true);
        let id = RequestId::Number(1);
        let (events_tx, _events_rx) = fresh_events_tx();
        let ctx = ctx_with(&status, &id, acp, &[], Some(&events_tx));

        let res = EventsHandler
            .handle("events/subscribe", json!({ "topics": ["bogus.topic"] }), ctx)
            .await;
        match res {
            Err(err) => assert_eq!(err.code, -32602),
            Ok(_) => panic!("unknown topic must reject"),
        }
    }

    #[tokio::test]
    async fn subscribe_without_events_tx_is_internal_error() {
        let acp = Arc::new(AcpAdapter::new(Config::default(), Arc::new(StatusBroadcast::new(true))));
        let status = StatusBroadcast::new(true);
        let id = RequestId::Number(1);
        let ctx = ctx_with(&status, &id, acp, &[], None);

        let res = EventsHandler.handle("events/subscribe", Value::Null, ctx).await;
        match res {
            Err(err) => assert_eq!(err.code, -32603),
            Ok(_) => panic!("must fail without events tx"),
        }
    }

    #[tokio::test]
    async fn unsubscribe_known_id_returns_unsubscribed_true() {
        let acp = Arc::new(AcpAdapter::new(Config::default(), Arc::new(StatusBroadcast::new(true))));
        let status = StatusBroadcast::new(true);
        let id = RequestId::Number(1);
        let known = vec!["sub-1".to_string()];
        let (events_tx, _events_rx) = fresh_events_tx();
        let ctx = ctx_with(&status, &id, acp, &known, Some(&events_tx));

        let outcome = EventsHandler
            .handle("events/unsubscribe", json!({ "subscriptionId": "sub-1" }), ctx)
            .await
            .unwrap();
        match outcome {
            HandlerOutcome::Reply(v) => {
                assert_eq!(v["unsubscribed"], true);
                assert_eq!(v["subscriptionId"], "sub-1");
            }
            _ => panic!("expected Reply"),
        }
    }

    #[tokio::test]
    async fn unsubscribe_unknown_id_returns_unsubscribed_false() {
        let acp = Arc::new(AcpAdapter::new(Config::default(), Arc::new(StatusBroadcast::new(true))));
        let status = StatusBroadcast::new(true);
        let id = RequestId::Number(1);
        let (events_tx, _events_rx) = fresh_events_tx();
        let ctx = ctx_with(&status, &id, acp, &[], Some(&events_tx));

        let outcome = EventsHandler
            .handle("events/unsubscribe", json!({ "subscriptionId": "ghost" }), ctx)
            .await
            .unwrap();
        match outcome {
            HandlerOutcome::Reply(v) => {
                assert_eq!(v["unsubscribed"], false);
                assert_eq!(v["subscriptionId"], "ghost");
            }
            _ => panic!("expected Reply"),
        }
    }

    #[tokio::test]
    async fn unknown_method_is_method_not_found() {
        let acp = Arc::new(AcpAdapter::new(Config::default(), Arc::new(StatusBroadcast::new(true))));
        let status = StatusBroadcast::new(true);
        let id = RequestId::Number(1);
        let ctx = ctx_with(&status, &id, acp, &[], None);

        let res = EventsHandler.handle("events/bogus", Value::Null, ctx).await;
        match res {
            Err(err) => assert_eq!(err.code, -32601),
            Ok(_) => panic!("unknown method must reject"),
        }
    }

    fn evt(instance_id: &str) -> InstanceEvent {
        InstanceEvent::State {
            agent_id: "a".into(),
            instance_id: instance_id.into(),
            session_id: None,
            state: InstanceState::Running,
        }
    }

    #[test]
    fn accept_no_filters_passes_everything() {
        let topics: HashSet<WireTopic> = HashSet::new();
        assert!(accept(&evt("id-1"), &topics, None));
    }

    #[test]
    fn accept_with_topic_filter_drops_non_matching() {
        let mut topics = HashSet::new();
        topics.insert(WireTopic::InstanceTranscript);
        assert!(!accept(&evt("id-1"), &topics, None));

        topics.insert(WireTopic::InstanceState);
        assert!(accept(&evt("id-1"), &topics, None));
    }

    #[test]
    fn accept_with_instance_filter_keeps_matching() {
        let topics: HashSet<WireTopic> = HashSet::new();
        assert!(accept(&evt("id-1"), &topics, Some("id-1")));
        assert!(!accept(&evt("id-1"), &topics, Some("id-2")));
    }

    #[test]
    fn accept_instance_filter_drops_registry_wide_events() {
        let topics: HashSet<WireTopic> = HashSet::new();
        let evt = InstanceEvent::InstancesChanged {
            instance_ids: vec![],
            focused_id: None,
        };
        assert!(!accept(&evt, &topics, Some("id-1")));
    }

    /// `build_notify_params` projects an `InstanceEvent` onto the
    /// wire-shape; verify the topic / instance_id / payload fields
    /// land correctly for an instance-bound event and a registry-wide
    /// event.
    #[test]
    fn build_notify_params_shapes_correctly() {
        let p = build_notify_params("sub-1", &evt("id-1")).unwrap();
        assert_eq!(p.subscription_id, "sub-1");
        assert_eq!(p.topic, WireTopic::InstanceState);
        assert_eq!(p.instance_id.as_deref(), Some("id-1"));

        let registry_evt = InstanceEvent::InstancesChanged {
            instance_ids: vec!["id-x".into()],
            focused_id: None,
        };
        let p = build_notify_params("sub-1", &registry_evt).unwrap();
        assert_eq!(p.topic, WireTopic::InstancesChanged);
        assert!(p.instance_id.is_none(), "registry-wide event has no instance id");
    }
}
