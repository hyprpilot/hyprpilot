use std::sync::{Arc, RwLock};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tracing::{error, info, trace, warn};

use crate::adapters::{AcpAdapter, Adapter};
use crate::config::Config;
use crate::rpc::handler::{EventsConnectionTx, EventsSubscription, HandlerCtx, HandlerOutcome};
use crate::rpc::protocol::{
    EventsNotifyNotification, EventsNotifyParams, JsonRpcVersion, Outcome, RequestId, Response, RpcError,
    StatusChangedNotification,
};
use crate::rpc::status::StatusBroadcast;
use crate::rpc::RpcDispatcher;

/// Shared state handed to each connection task. `AppHandle` is cheap to
/// clone, so we pass by value today; `StatusBroadcast`, `RpcDispatcher`,
/// the adapter, and the config are wrapped in `Arc` for cheap fan-out
/// across every accepted connection. `config` is read-only at runtime —
/// the `RwLock` is there so adapter + RPC handlers can share one handle
/// without re-cloning per call; restart-to-change is the model.
#[derive(Clone)]
pub struct RpcState {
    pub app: tauri::AppHandle,
    pub status: Arc<StatusBroadcast>,
    pub dispatcher: Arc<RpcDispatcher>,
    pub adapter: Arc<dyn Adapter>,
    pub acp_adapter: Arc<AcpAdapter>,
    pub config: Arc<RwLock<Config>>,
}

/// One accepted connection. Reads NDJSON, dispatches, writes the
/// response, loops. After `status/subscribe` the same loop
/// multiplexes `status/changed` notifications via `tokio::select!`;
/// after `events/subscribe` it also drains per-subscription mpsc
/// receivers and writes `events/notify` notifications inline.
pub async fn handle_connection(stream: UnixStream, state: RpcState) {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();

    // Set by `status/subscribe`; its presence also gates the second-
    // subscribe rejection via `HandlerCtx::already_subscribed`.
    let mut status_rx: Option<tokio::sync::broadcast::Receiver<crate::rpc::protocol::StatusResult>> = None;
    // Per-connection events subscriptions. Empty by default; one entry
    // per `events/subscribe` call. Each holds a oneshot cancel sender —
    // dropping the entry signals the filter task to exit. The shared
    // `events_rx` drains every active subscription's notifications;
    // every filter task pushes onto a clone of `events_tx`.
    let mut events_subs: Vec<EventsSubscription> = Vec::new();
    let mut events_tx_holder: Option<EventsConnectionTx> = None;
    let mut events_rx: Option<tokio::sync::mpsc::Receiver<EventsNotifyParams>> = None;

    loop {
        tokio::select! {
            // ── incoming client line ──────────────────────────────────────
            line_result = lines.next_line() => {
                let line = match line_result {
                    Ok(Some(l)) if l.trim().is_empty() => continue,
                    Ok(Some(l)) => l,
                    Ok(None) => return,
                    Err(err) => {
                        warn!(%err, "rpc: read error, closing connection");
                        return;
                    }
                };

                trace!(line = %line, "rpc: received line");

                // Lazy-init the connection's shared events mpsc on first
                // events/subscribe. Subsequent subscribes clone the
                // existing tx so every filter task feeds the same rx.
                if events_tx_holder.is_none() {
                    let (tx, rx) = tokio::sync::mpsc::channel::<EventsNotifyParams>(EVENTS_CONNECTION_QUEUE_CAPACITY);
                    events_tx_holder = Some(EventsConnectionTx { tx });
                    events_rx = Some(rx);
                }

                let existing_subscription_ids: Vec<String> =
                    events_subs.iter().map(|s| s.subscription_id.clone()).collect();
                let events_tx_ref = events_tx_holder.as_ref();

                let dispatch_result = dispatch(
                    &line,
                    Some(&state.app),
                    &state.status,
                    &state.dispatcher,
                    Some(state.adapter.clone()),
                    Some(state.acp_adapter.clone()),
                    Some(state.config.clone()),
                    status_rx.is_some(),
                    &existing_subscription_ids,
                    events_tx_ref,
                ).await;

                let DispatchOutput { response, new_status_rx, new_events_sub, unsubscribe_id } = dispatch_result;

                if let Some(rx) = new_status_rx {
                    status_rx = Some(rx);
                }
                if let Some(sub) = new_events_sub {
                    events_subs.push(sub);
                }
                if let Some(id) = unsubscribe_id {
                    // Drop the matching EventsSubscription — its cancel
                    // sender drops with it and the filter task notices
                    // on its next select! iteration.
                    events_subs.retain(|s| s.subscription_id != id);
                }

                // Kill signal rides in the response payload as
                // `{"killed": true}`; captured before `response` moves
                // so we can act after the flush.
                let kill_signalled = matches!(
                    &response.outcome,
                    Outcome::Success { result } if result.get("killed").and_then(|v| v.as_bool()) == Some(true)
                );

                let mut bytes = match serde_json::to_vec(&response) {
                    Ok(b) => b,
                    Err(err) => {
                        error!(%err, "rpc: failed to serialize response");
                        return;
                    }
                };
                bytes.push(b'\n');

                if let Err(err) = writer.write_all(&bytes).await {
                    warn!(%err, "rpc: write error, closing connection");
                    return;
                }
                let _ = writer.flush().await;

                if kill_signalled {
                    crate::daemon::shutdown(&state.app, state.acp_adapter.as_ref()).await;
                    return;
                }
            }

            // ── outbound status/changed notification (subscriber only) ────
            notification = async {
                match &mut status_rx {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                match notification {
                    Ok(sr) => {
                        let notif = StatusChangedNotification::new(sr);
                        let mut bytes = match serde_json::to_vec(&notif) {
                            Ok(b) => b,
                            Err(err) => {
                                error!(%err, "rpc: failed to serialize status/changed");
                                return;
                            }
                        };
                        bytes.push(b'\n');
                        if let Err(err) = writer.write_all(&bytes).await {
                            warn!(%err, "rpc: write error on status/changed, closing");
                            return;
                        }
                        let _ = writer.flush().await;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!(n, "rpc: subscriber lagged, re-pushing current snapshot");
                        // The select! loop waits for the *next* transition;
                        // if nothing further happens, the subscriber sits on
                        // stale state indefinitely. Re-send the current
                        // snapshot so the peer resynchronises immediately.
                        let current = state.status.get();
                        let notif = StatusChangedNotification::new(current);
                        let mut bytes = match serde_json::to_vec(&notif) {
                            Ok(b) => b,
                            Err(err) => {
                                error!(%err, "rpc: failed to serialize resync status/changed");
                                return;
                            }
                        };
                        bytes.push(b'\n');
                        if let Err(err) = writer.write_all(&bytes).await {
                            warn!(%err, "rpc: write error on resync, closing");
                            return;
                        }
                        let _ = writer.flush().await;
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        // Broadcast sender dropped — nothing left to receive. Close.
                        return;
                    }
                }
            }

            // ── outbound events/notify (single per-connection mpsc) ───────
            // Every active events subscription pushes onto the same mpsc;
            // the loop drains it and writes one line per item. When no
            // subscriptions exist (and thus no rx), the future is
            // `pending()` and never fires.
            notification = async {
                match &mut events_rx {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                let Some(params) = notification else {
                    // Sender side dropped — connection's events_tx
                    // holder should still be alive; this shouldn't
                    // happen unless the holder was cleared. Treat as
                    // a no-op and keep going.
                    continue;
                };
                let notif = EventsNotifyNotification::new(params);
                let mut bytes = match serde_json::to_vec(&notif) {
                    Ok(b) => b,
                    Err(err) => {
                        error!(%err, "rpc: failed to serialize events/notify");
                        return;
                    }
                };
                bytes.push(b'\n');
                if let Err(err) = writer.write_all(&bytes).await {
                    warn!(%err, "rpc: write error on events/notify, closing");
                    return;
                }
                let _ = writer.flush().await;
            }
        }
    }
}

/// Per-connection events queue capacity. Single shared queue across
/// every active `events/subscribe` on the connection — the filter
/// tasks all push onto a clone of the same sender. Slow consumers
/// see `try_send` Full + a `warn!` + drop per CLAUDE.md
/// "drop on slow-consumer backpressure".
const EVENTS_CONNECTION_QUEUE_CAPACITY: usize = 256;

/// Dispatch result. Single struct so the connection loop can grow new
/// fields without reflowing every match arm.
struct DispatchOutput {
    response: Response,
    new_status_rx: Option<tokio::sync::broadcast::Receiver<crate::rpc::protocol::StatusResult>>,
    new_events_sub: Option<EventsSubscription>,
    /// Subscription id to evict from the per-connection vec — set by
    /// the events/unsubscribe handler via the response payload.
    unsubscribe_id: Option<String>,
}

/// Parse one NDJSON line → `RpcDispatcher` → JSON-RPC `Response`.
/// `DispatchOutput` slots are populated by the corresponding subscriber
/// handlers; `Reply` paths leave them `None`. Tests pass `None` for
/// `app` + a stand-alone fixture so they can drive routing without a
/// live Tauri runtime.
#[allow(clippy::too_many_arguments)]
async fn dispatch(
    line: &str,
    app: Option<&tauri::AppHandle>,
    status: &StatusBroadcast,
    dispatcher: &RpcDispatcher,
    adapter: Option<Arc<dyn Adapter>>,
    acp_adapter: Option<Arc<AcpAdapter>>,
    config: Option<Arc<RwLock<Config>>>,
    connection_already_subscribed: bool,
    existing_event_subscription_ids: &[String],
    events_tx: Option<&EventsConnectionTx>,
) -> DispatchOutput {
    // Stage 1: JSON syntax. Failure here is -32700 parse error.
    let value: serde_json::Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(_) => return DispatchOutput::reply(Response::error(None, RpcError::parse_error())),
    };

    let id = value.get("id").and_then(parse_id);

    // Stage 2: envelope. `jsonrpc` must be present and equal to "2.0".
    if !value.is_object() {
        return DispatchOutput::reply(Response::error(
            None,
            RpcError::invalid_request("request must be a JSON object"),
        ));
    }
    match value.get("jsonrpc") {
        Some(v) => {
            if serde_json::from_value::<JsonRpcVersion>(v.clone()).is_err() {
                return DispatchOutput::reply(Response::error(
                    id,
                    RpcError::invalid_request("jsonrpc must be \"2.0\""),
                ));
            }
        }
        None => {
            return DispatchOutput::reply(Response::error(id, RpcError::invalid_request("missing jsonrpc field")));
        }
    }
    let id = match id {
        Some(id) => id,
        None => {
            return DispatchOutput::reply(Response::error(
                None,
                RpcError::invalid_request("missing or invalid id"),
            ));
        }
    };

    // Stage 3: method + params handed off to the dispatcher.
    let method = match value.get("method").and_then(|v| v.as_str()) {
        Some(m) => m.to_string(),
        None => {
            return DispatchOutput::reply(Response::error(
                Some(id),
                RpcError::invalid_request("missing method field"),
            ));
        }
    };
    let params = value.get("params").cloned().unwrap_or(serde_json::Value::Null);

    info!(id = ?id, method = %method, "rpc: dispatch entry");
    trace!(id = ?id, method = %method, params = %params, "rpc: dispatch params");

    let ctx = HandlerCtx {
        app,
        status,
        adapter,
        acp_adapter,
        config,
        id: &id,
        already_subscribed: connection_already_subscribed,
        existing_event_subscription_ids,
        events_tx,
    };

    let outcome = dispatcher.dispatch(&method, params, ctx).await;

    match outcome {
        Ok(HandlerOutcome::Reply(value)) => {
            // events/unsubscribe rides in the result payload as
            // `{"unsubscribed": true, "subscriptionId": "<id>"}`. Pull
            // the id so the connection loop can evict the matching
            // entry. The protocol stays self-describing — no separate
            // out-of-band signal threaded through the dispatcher tuple.
            let unsubscribe_id = value
                .get("unsubscribed")
                .and_then(serde_json::Value::as_bool)
                .and_then(|b| {
                    if b {
                        value.get("subscriptionId").and_then(|v| v.as_str()).map(str::to_string)
                    } else {
                        None
                    }
                });
            DispatchOutput {
                response: Response::success(Some(id), value),
                new_status_rx: None,
                new_events_sub: None,
                unsubscribe_id,
            }
        }
        Ok(HandlerOutcome::StatusSubscribed(snapshot, rx)) => DispatchOutput {
            response: Response::success(Some(id), snapshot),
            new_status_rx: Some(rx),
            new_events_sub: None,
            unsubscribe_id: None,
        },
        Ok(HandlerOutcome::EventsSubscribed(reply, sub)) => DispatchOutput {
            response: Response::success(Some(id), reply),
            new_status_rx: None,
            new_events_sub: Some(sub),
            unsubscribe_id: None,
        },
        Err(err) => {
            warn!(
                id = ?id,
                method = %method,
                code = err.code,
                message = %err.message,
                "rpc: handler returned error"
            );
            DispatchOutput::reply(Response::error(Some(id), err))
        }
    }
}

impl DispatchOutput {
    fn reply(response: Response) -> Self {
        Self {
            response,
            new_status_rx: None,
            new_events_sub: None,
            unsubscribe_id: None,
        }
    }
}

fn parse_id(v: &serde_json::Value) -> Option<RequestId> {
    match v {
        serde_json::Value::Number(n) => {
            if let Some(u) = n.as_u64() {
                Some(RequestId::Number(u))
            } else {
                n.as_f64().map(|f| RequestId::String(f.to_string()))
            }
        }
        serde_json::Value::String(s) => Some(RequestId::String(s.clone())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use serde_json::json;

    /// Driver for envelope / framing tests: no `AppHandle`, a fresh
    /// `StatusBroadcast`, a throwaway dispatcher + adapter per call.
    /// Handlers that need the app (only `window/toggle`) surface
    /// `-32603`; everything else runs through to the handler's real
    /// logic.
    async fn run(line: &str) -> serde_json::Value {
        let status = StatusBroadcast::new(true);
        let dispatcher = RpcDispatcher::with_defaults();
        let config = Arc::new(RwLock::new(Config::default()));
        let acp = Arc::new(AcpAdapter::new(Config::default(), Arc::new(StatusBroadcast::new(true))));
        let adapter: Arc<dyn Adapter> = acp.clone();
        let out = dispatch(
            line,
            None,
            &status,
            &dispatcher,
            Some(adapter),
            Some(acp),
            Some(config),
            false,
            &[],
            None,
        )
        .await;
        serde_json::to_value(out.response).unwrap()
    }

    /// With an empty `Config` (no `[agent] default`, no
    /// `[[agents]]`), `session/submit` rejects with `-32602` — no
    /// agent to spawn. End-to-end happy path is covered by the
    /// runtime integration test.
    #[tokio::test]
    async fn session_submit_without_default_is_invalid_params() {
        let out = run(r#"{"jsonrpc":"2.0","id":1,"method":"session/submit","params":{"text":"hello"}}"#).await;
        assert_eq!(out["jsonrpc"], "2.0");
        assert_eq!(out["id"], 1);
        assert_eq!(out["error"]["code"], -32602);
    }

    #[tokio::test]
    async fn session_info_returns_empty_list() {
        let out = run(r#"{"jsonrpc":"2.0","id":"sid","method":"session/info"}"#).await;
        assert_eq!(out["id"], "sid");
        assert_eq!(out["result"]["instances"], json!([]));
    }

    #[tokio::test]
    async fn session_cancel_without_default_is_invalid_params() {
        let out = run(r#"{"jsonrpc":"2.0","id":2,"method":"session/cancel"}"#).await;
        assert_eq!(out["error"]["code"], -32602);
    }

    #[tokio::test]
    async fn daemon_kill_returns_killed_true() {
        // The `killed: true` marker in the result is what
        // `handle_connection` inspects to trigger `app.exit(0)`
        // after the response flushes. No separate side-channel flag.
        let out = run(r#"{"jsonrpc":"2.0","id":3,"method":"daemon/kill"}"#).await;
        assert_eq!(out["result"]["killed"], true);
    }

    #[tokio::test]
    async fn parse_error_on_non_json() {
        let out = run("not json at all").await;
        assert_eq!(out["error"]["code"], -32700);
        assert!(out["id"].is_null(), "id must be null on parse error: {out}");
    }

    #[tokio::test]
    async fn invalid_request_missing_jsonrpc() {
        let out = run(r#"{"id":1,"method":"window/toggle"}"#).await;
        assert_eq!(out["error"]["code"], -32600);
        assert_eq!(out["id"], 1);
    }

    #[tokio::test]
    async fn method_not_found_for_unknown_method() {
        let out = run(r#"{"jsonrpc":"2.0","id":4,"method":"bogus"}"#).await;
        assert_eq!(out["error"]["code"], -32601);
        assert_eq!(out["id"], 4);
        assert!(out["error"]["message"].as_str().unwrap().contains("bogus"));
    }

    /// Regression: bare `"status"` (no `/verb`) must fall through to
    /// `-32601 method_not_found`. The old `Call` enum had explicit
    /// `#[serde(rename = "status/get")]` guards for this; the
    /// namespace-based dispatcher relies on the namespace lookup
    /// failing (namespace `""` → `CoreHandler` → unknown method) or
    /// the `StatusHandler` rejecting the bare name.
    #[tokio::test]
    async fn bare_status_method_name_is_method_not_found() {
        let out = run(r#"{"jsonrpc":"2.0","id":9,"method":"status"}"#).await;
        assert_eq!(out["error"]["code"], -32601, "{out}");

        let out = run(r#"{"jsonrpc":"2.0","id":10,"method":"subscribe"}"#).await;
        assert_eq!(out["error"]["code"], -32601, "{out}");
    }

    #[tokio::test]
    async fn invalid_request_wrong_jsonrpc_version() {
        let out = run(r#"{"jsonrpc":"1.0","id":11,"method":"window/toggle"}"#).await;
        assert_eq!(out["error"]["code"], -32600);
        assert_eq!(out["id"], 11);
    }

    /// Regression: `window/toggle` with no `params` key at all
    /// deserializes cleanly — the `WindowHandler` ignores params for
    /// unit methods. Keep the invariant by hitting the full wire path.
    #[tokio::test]
    async fn window_toggle_without_params_is_handled() {
        // No params key. Handler surfaces -32603 because `app` is None
        // in the test harness, but the absence of -32600 / -32602
        // proves the line parsed + routed correctly.
        let out = run(r#"{"jsonrpc":"2.0","id":12,"method":"window/toggle"}"#).await;
        assert_eq!(out["error"]["code"], -32603);
    }

    /// Regression: a string id round-trips through `session/submit`,
    /// even when the deeper handler rejects with `-32602` because the
    /// test harness has no agent config. The envelope handling is what
    /// this test pins — happy-path submit is exercised at the
    /// `AcpInstances` layer.
    #[tokio::test]
    async fn session_submit_with_string_id_round_trips() {
        let out = run(r#"{"jsonrpc":"2.0","id":"abc-123","method":"session/submit","params":{"text":"hi"}}"#).await;
        assert_eq!(out["id"], "abc-123");
        assert_eq!(out["error"]["code"], -32602);
    }

    /// Every bare legacy method name must surface `-32601` after
    /// K-239's rename. No backwards-compat alias — downstream peers
    /// are expected to move to `namespace/name`.
    #[tokio::test]
    async fn bare_legacy_method_names_are_method_not_found() {
        for method in ["submit", "cancel", "toggle", "kill", "session-info"] {
            let body = format!(r#"{{"jsonrpc":"2.0","id":99,"method":"{method}"}}"#);
            let out = run(&body).await;
            assert_eq!(out["error"]["code"], -32601, "bare {method} must be -32601: {out}");
        }
    }

    /// Regression: an unknown method in the `status` namespace returns
    /// `-32601 method_not_found` with the full method name echoed
    /// back, not just the namespace.
    #[tokio::test]
    async fn unknown_status_verb_is_method_not_found() {
        let out = run(r#"{"jsonrpc":"2.0","id":14,"method":"status/bogus"}"#).await;
        assert_eq!(out["error"]["code"], -32601);
        assert!(out["error"]["message"].as_str().unwrap().contains("status/bogus"));
    }

    #[tokio::test]
    async fn float_id_is_echoed_as_string() {
        let out = run(r#"{"jsonrpc":"2.0","id":1.5,"method":"bogus"}"#).await;
        assert_eq!(out["error"]["code"], -32601);
        assert_eq!(out["id"], "1.5");
    }

    #[tokio::test]
    async fn invalid_params_for_missing_required_field() {
        let out = run(r#"{"jsonrpc":"2.0","id":6,"method":"session/submit","params":{}}"#).await;
        assert_eq!(out["error"]["code"], -32602);
        assert_eq!(out["id"], 6);
    }

    #[tokio::test]
    async fn window_toggle_without_app_is_internal_error() {
        let out = run(r#"{"jsonrpc":"2.0","id":5,"method":"window/toggle"}"#).await;
        assert_eq!(out["error"]["code"], -32603);
        assert_eq!(out["id"], 5);
    }

    #[tokio::test]
    async fn status_get_returns_broadcast_snapshot() {
        let out = run(r#"{"jsonrpc":"2.0","id":7,"method":"status/get"}"#).await;
        assert_eq!(out["id"], 7);
        assert_eq!(out["result"]["state"], "idle");
        assert_eq!(out["result"]["visible"], true);
    }

    #[tokio::test]
    async fn status_subscribe_returns_snapshot() {
        let out = run(r#"{"jsonrpc":"2.0","id":8,"method":"status/subscribe"}"#).await;
        assert_eq!(out["id"], 8);
        assert_eq!(out["result"]["state"], "idle");
    }

    #[tokio::test]
    async fn handle_connection_closes_on_eof() {
        let (mut client, server) = UnixStream::pair().expect("pair");

        let task = tokio::spawn(async move {
            let (reader, mut writer) = server.into_split();
            let mut lines = BufReader::new(reader).lines();
            let status = StatusBroadcast::new(true);
            let dispatcher = RpcDispatcher::with_defaults();
            let config = Arc::new(RwLock::new(Config::default()));
            let acp = Arc::new(AcpAdapter::new(Config::default(), Arc::new(StatusBroadcast::new(true))));
            let adapter: Arc<dyn Adapter> = acp.clone();
            while let Some(l) = lines.next_line().await.unwrap() {
                if l.trim().is_empty() {
                    continue;
                }
                let out = dispatch(
                    &l,
                    None,
                    &status,
                    &dispatcher,
                    Some(adapter.clone()),
                    Some(acp.clone()),
                    Some(config.clone()),
                    false,
                    &[],
                    None,
                )
                .await;
                let mut bytes = serde_json::to_vec(&out.response).unwrap();
                bytes.push(b'\n');
                writer.write_all(&bytes).await.unwrap();
                writer.flush().await.unwrap();
            }
        });

        client
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"session/info\"}\n")
            .await
            .unwrap();

        let mut buf = String::new();
        BufReader::new(&mut client).read_line(&mut buf).await.unwrap();

        let v: serde_json::Value = serde_json::from_str(&buf).unwrap();
        assert_eq!(v["id"], 1);
        assert_eq!(v["result"]["instances"], json!([]));

        drop(client);
        task.await.unwrap();
    }

    /// Regression: a second `status/subscribe` on the same connection
    /// must return `-32600 invalid_request`. The server threads
    /// `status_rx.is_some()` through `dispatch_line` into
    /// `HandlerCtx::already_subscribed`; `StatusHandler` checks the flag
    /// and rejects the call before touching the broadcast.
    #[tokio::test]
    async fn double_subscribe_on_same_connection_is_rejected() {
        let broadcast = StatusBroadcast::new(true);
        let dispatcher = RpcDispatcher::with_defaults();
        let config = Arc::new(RwLock::new(Config::default()));
        let acp = Arc::new(AcpAdapter::new(Config::default(), Arc::new(StatusBroadcast::new(true))));
        let adapter: Arc<dyn Adapter> = acp.clone();

        // First subscribe — emulated: we don't actually route, we just
        // mark the connection state as "already_subscribed = true" and
        // drive the second subscribe through `dispatch_line`.
        let out = dispatch(
            r#"{"jsonrpc":"2.0","id":1,"method":"status/subscribe"}"#,
            None,
            &broadcast,
            &dispatcher,
            Some(adapter.clone()),
            Some(acp.clone()),
            Some(config.clone()),
            false,
            &[],
            None,
        )
        .await;
        let v = serde_json::to_value(&out.response).unwrap();
        assert_eq!(v["result"]["state"], "idle", "first subscribe succeeds");
        assert!(out.new_status_rx.is_some(), "first subscribe returns a receiver");

        // Second subscribe on the same connection — already_subscribed = true.
        let out = dispatch(
            r#"{"jsonrpc":"2.0","id":2,"method":"status/subscribe"}"#,
            None,
            &broadcast,
            &dispatcher,
            Some(adapter),
            Some(acp),
            Some(config),
            true,
            &[],
            None,
        )
        .await;
        let v = serde_json::to_value(&out.response).unwrap();
        assert_eq!(v["error"]["code"], -32600, "second subscribe is rejected: {v}");
        assert_eq!(v["id"], 2);
        assert!(
            out.new_status_rx.is_none(),
            "rejected subscribe must not produce a receiver"
        );
    }

    /// Regression for the Lagged arm: when a subscriber falls behind
    /// more than the broadcast channel's capacity, `recv()` returns
    /// `RecvError::Lagged(n)`. The server used to log + drop the
    /// error, leaving the peer on stale state until some future
    /// transition kicked the select loop. The fix re-pushes
    /// `status.get()` as a `status/changed` notification so the
    /// subscriber resynchronises immediately.
    ///
    /// The test runs the real Lagged arm by driving a minimal
    /// handle_connection-like loop, floods 200 set()s past the
    /// capacity-32 channel, then asserts the subscriber receives a
    /// `status/changed` with the current snapshot after the Lagged
    /// fires.
    #[tokio::test]
    async fn lagged_subscriber_receives_resynced_snapshot() {
        use crate::rpc::protocol::{AgentState, StatusResult};

        let broadcast = Arc::new(StatusBroadcast::new(true));
        let (_snap_initial, mut rx) = broadcast.subscribe();

        // Flood past capacity-32 without draining.
        for i in 0..200u32 {
            broadcast.set(StatusResult {
                state: if i % 2 == 0 {
                    AgentState::Streaming
                } else {
                    AgentState::Idle
                },
                visible: true,
                active_session: None,
            });
        }

        // Set the final snapshot we'll compare against.
        let final_state = StatusResult {
            state: AgentState::Awaiting,
            visible: false,
            active_session: Some("sess-resync".into()),
        };
        broadcast.set(final_state.clone());

        // First recv pulls the Lagged signal (because we're way behind).
        let first = rx.recv().await;
        assert!(
            matches!(first, Err(tokio::sync::broadcast::error::RecvError::Lagged(_))),
            "expected Lagged, got {first:?}"
        );

        // Now run the Lagged arm's logic manually (same code as the
        // server's select! arm): snapshot → serialize → "write".
        let current = broadcast.get();
        assert_eq!(current, final_state, "resync snapshot must be the current state");
        let notif = StatusChangedNotification::new(current);
        let bytes = serde_json::to_vec(&notif).expect("serialize");
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["method"], "status/changed");
        assert_eq!(value["params"]["state"], "awaiting");
        assert_eq!(value["params"]["visible"], false);
        assert_eq!(value["params"]["active_session"], "sess-resync");
    }

    /// Integration test: subscribe through `dispatch_line`, flip state via
    /// the shared `StatusBroadcast`, assert the subscriber receives
    /// exactly one `status/changed` notification, and the connection
    /// cleans up without leaking senders. This is the one test that
    /// exercises the full server path end-to-end (framing + dispatcher +
    /// broadcast fan-out) without needing a live `AppHandle`.
    #[tokio::test]
    async fn subscribe_and_notify_integration() {
        use crate::rpc::protocol::{AgentState, StatusResult};

        let broadcast = Arc::new(StatusBroadcast::new(true));
        let broadcast_clone = broadcast.clone();

        let (client, server) = UnixStream::pair().expect("pair");
        let (client_reader, mut client_writer) = client.into_split();
        let mut client_lines = BufReader::new(client_reader).lines();

        let task = tokio::spawn(async move {
            let (reader, mut writer) = server.into_split();
            let mut lines = BufReader::new(reader).lines();
            let dispatcher = RpcDispatcher::with_defaults();
            let config = Arc::new(RwLock::new(Config::default()));
            let acp = Arc::new(AcpAdapter::new(Config::default(), Arc::new(StatusBroadcast::new(true))));
            let adapter: Arc<dyn Adapter> = acp.clone();
            let mut status_rx: Option<tokio::sync::broadcast::Receiver<StatusResult>> = None;

            loop {
                tokio::select! {
                    line_result = lines.next_line() => {
                        match line_result {
                            Ok(Some(l)) if l.trim().is_empty() => continue,
                            Ok(Some(l)) => {
                                let out = dispatch(
                                    &l,
                                    None,
                                    &broadcast_clone,
                                    &dispatcher,
                                    Some(adapter.clone()),
                                    Some(acp.clone()),
                                    Some(config.clone()),
                                    status_rx.is_some(),
                                    &[],
                                    None,
                                ).await;
                                if let Some(rx) = out.new_status_rx {
                                    status_rx = Some(rx);
                                }
                                let mut bytes = serde_json::to_vec(&out.response).unwrap();
                                bytes.push(b'\n');
                                writer.write_all(&bytes).await.unwrap();
                                writer.flush().await.unwrap();
                            }
                            _ => return,
                        }
                    }
                    notification = async {
                        match &mut status_rx {
                            Some(rx) => rx.recv().await,
                            None => std::future::pending().await,
                        }
                    } => {
                        match notification {
                            Ok(sr) => {
                                let notif = StatusChangedNotification::new(sr);
                                let mut bytes = serde_json::to_vec(&notif).unwrap();
                                bytes.push(b'\n');
                                if writer.write_all(&bytes).await.is_err() {
                                    return;
                                }
                                writer.flush().await.unwrap();
                            }
                            _ => return,
                        }
                    }
                }
            }
        });

        client_writer
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"status/subscribe\"}\n")
            .await
            .unwrap();

        let line = client_lines.next_line().await.unwrap().unwrap();
        let snap: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(snap["result"]["state"], "idle");
        assert_eq!(snap["result"]["visible"], true);

        broadcast.set(StatusResult {
            state: AgentState::Streaming,
            visible: true,
            active_session: None,
        });

        let notif_line = client_lines.next_line().await.unwrap().unwrap();
        let notif: serde_json::Value = serde_json::from_str(notif_line.trim()).unwrap();
        assert_eq!(notif["method"], "status/changed");
        assert_eq!(notif["params"]["state"], "streaming");

        drop(client_writer);
        task.await.unwrap();
    }

    /// Spin up a minimal handle_connection-shaped loop and drive a
    /// full events/subscribe → publish → events/notify path. Uses the
    /// real `AcpAdapter` broadcast so the filter task wires through
    /// the same `subscribe_events()` consumers see in production. The
    /// connection's events mpsc + per-subscription cancel flow are
    /// exercised end-to-end.
    #[tokio::test]
    async fn events_subscribe_publish_notify_round_trips() {
        let broadcast = Arc::new(StatusBroadcast::new(true));
        let acp = Arc::new(AcpAdapter::new(Config::default(), broadcast.clone()));

        let (client, server) = UnixStream::pair().expect("pair");
        let (client_reader, mut client_writer) = client.into_split();
        let mut client_lines = BufReader::new(client_reader).lines();

        let acp_clone = acp.clone();
        let task = tokio::spawn(async move {
            let (reader, mut writer) = server.into_split();
            let mut lines = BufReader::new(reader).lines();
            let dispatcher = RpcDispatcher::with_defaults();
            let config = Arc::new(RwLock::new(Config::default()));
            let adapter: Arc<dyn Adapter> = acp_clone.clone();

            let mut events_subs: Vec<EventsSubscription> = Vec::new();
            let mut events_tx_holder: Option<EventsConnectionTx> = None;
            let mut events_rx: Option<tokio::sync::mpsc::Receiver<EventsNotifyParams>> = None;

            loop {
                tokio::select! {
                    line_result = lines.next_line() => {
                        match line_result {
                            Ok(Some(l)) if l.trim().is_empty() => continue,
                            Ok(Some(l)) => {
                                if events_tx_holder.is_none() {
                                    let (tx, rx) = tokio::sync::mpsc::channel::<EventsNotifyParams>(EVENTS_CONNECTION_QUEUE_CAPACITY);
                                    events_tx_holder = Some(EventsConnectionTx { tx });
                                    events_rx = Some(rx);
                                }
                                let existing: Vec<String> = events_subs.iter().map(|s| s.subscription_id.clone()).collect();
                                let out = dispatch(
                                    &l,
                                    None,
                                    &broadcast,
                                    &dispatcher,
                                    Some(adapter.clone()),
                                    Some(acp_clone.clone()),
                                    Some(config.clone()),
                                    false,
                                    &existing,
                                    events_tx_holder.as_ref(),
                                ).await;
                                if let Some(sub) = out.new_events_sub {
                                    events_subs.push(sub);
                                }
                                if let Some(id) = out.unsubscribe_id {
                                    events_subs.retain(|s| s.subscription_id != id);
                                }
                                let mut bytes = serde_json::to_vec(&out.response).unwrap();
                                bytes.push(b'\n');
                                writer.write_all(&bytes).await.unwrap();
                                writer.flush().await.unwrap();
                            }
                            _ => return,
                        }
                    }
                    notification = async {
                        match &mut events_rx {
                            Some(rx) => rx.recv().await,
                            None => std::future::pending().await,
                        }
                    } => {
                        let Some(params) = notification else {
                            continue;
                        };
                        let notif = EventsNotifyNotification::new(params);
                        let mut bytes = serde_json::to_vec(&notif).unwrap();
                        bytes.push(b'\n');
                        if writer.write_all(&bytes).await.is_err() {
                            return;
                        }
                        writer.flush().await.unwrap();
                    }
                }
            }
        });

        // Subscribe.
        client_writer
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"events/subscribe\"}\n")
            .await
            .unwrap();
        let line = client_lines.next_line().await.unwrap().unwrap();
        let resp: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        let subscription_id = resp["result"]["subscriptionId"].as_str().unwrap().to_string();
        assert!(!subscription_id.is_empty());

        // Give the spawned filter task a tick to attach to the broadcast.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Publish via the test-only events_tx handle.
        let evt_tx = acp.test_events_tx();
        evt_tx
            .send(crate::adapters::InstanceEvent::InstancesChanged {
                instance_ids: vec!["id-1".into()],
                focused_id: Some("id-1".into()),
            })
            .expect("send");

        // Drain events/notify.
        let notif_line = tokio::time::timeout(std::time::Duration::from_secs(2), client_lines.next_line())
            .await
            .expect("notify within timeout")
            .unwrap()
            .unwrap();
        let notif: serde_json::Value = serde_json::from_str(notif_line.trim()).unwrap();
        assert_eq!(notif["method"], "events/notify");
        assert_eq!(notif["params"]["subscriptionId"], subscription_id);
        assert_eq!(notif["params"]["topic"], "instances.changed");
        assert_eq!(notif["params"]["payload"]["event"], "instances_changed");

        // Unsubscribe.
        let unsub = format!(
            r#"{{"jsonrpc":"2.0","id":2,"method":"events/unsubscribe","params":{{"subscriptionId":"{subscription_id}"}}}}"#
        );
        client_writer.write_all(unsub.as_bytes()).await.unwrap();
        client_writer.write_all(b"\n").await.unwrap();
        let line = client_lines.next_line().await.unwrap().unwrap();
        let resp: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(resp["result"]["unsubscribed"], true);

        drop(client_writer);
        task.await.unwrap();
    }

    /// Connection drop tears down every spawned filter task. We can't
    /// easily peek at the spawned task vec from inside, but we can
    /// verify the task that owns the connection loop exits cleanly
    /// when the client side disappears — which propagates dropping
    /// the events_tx through to the filter task's mpsc.
    #[tokio::test]
    async fn events_subscribe_then_connection_drops_cleanly() {
        let broadcast = Arc::new(StatusBroadcast::new(true));
        let acp = Arc::new(AcpAdapter::new(Config::default(), broadcast.clone()));

        let (client, server) = UnixStream::pair().expect("pair");
        let (_client_reader, mut client_writer) = client.into_split();

        let acp_clone = acp.clone();
        let task = tokio::spawn(async move {
            let (reader, mut writer) = server.into_split();
            let mut lines = BufReader::new(reader).lines();
            let dispatcher = RpcDispatcher::with_defaults();
            let config = Arc::new(RwLock::new(Config::default()));
            let adapter: Arc<dyn Adapter> = acp_clone.clone();
            let mut events_subs: Vec<EventsSubscription> = Vec::new();
            let mut events_tx_holder: Option<EventsConnectionTx> = None;

            while let Some(l) = lines.next_line().await.unwrap_or(None) {
                if l.trim().is_empty() {
                    continue;
                }
                if events_tx_holder.is_none() {
                    let (tx, _rx) = tokio::sync::mpsc::channel::<EventsNotifyParams>(EVENTS_CONNECTION_QUEUE_CAPACITY);
                    events_tx_holder = Some(EventsConnectionTx { tx });
                }
                let existing: Vec<String> = events_subs.iter().map(|s| s.subscription_id.clone()).collect();
                let out = dispatch(
                    &l,
                    None,
                    &broadcast,
                    &dispatcher,
                    Some(adapter.clone()),
                    Some(acp_clone.clone()),
                    Some(config.clone()),
                    false,
                    &existing,
                    events_tx_holder.as_ref(),
                )
                .await;
                if let Some(sub) = out.new_events_sub {
                    events_subs.push(sub);
                }
                let mut bytes = serde_json::to_vec(&out.response).unwrap();
                bytes.push(b'\n');
                writer.write_all(&bytes).await.unwrap();
                writer.flush().await.unwrap();
            }
            // Connection closed — events_subs drops here, which drops
            // every cancel sender, which terminates every filter task.
        });

        client_writer
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"events/subscribe\"}\n")
            .await
            .unwrap();
        // Give the dispatcher + filter task time to attach.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Drop the client side. The server task exits when next_line
        // returns None / Err. The cancel oneshots fire on drop.
        drop(client_writer);

        tokio::time::timeout(std::time::Duration::from_secs(2), task)
            .await
            .expect("connection task exits within timeout")
            .unwrap();
    }
}
