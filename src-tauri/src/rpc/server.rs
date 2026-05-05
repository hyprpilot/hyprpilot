use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::Instant;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tracing::{error, info, trace, warn};

use crate::adapters::Adapter;
use crate::config::Config;
use crate::rpc::handler::{HandlerCtx, HandlerOutcome};
use crate::rpc::protocol::{JsonRpcVersion, Outcome, RequestId, Response, RpcError, StatusChangedNotification};
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
    pub config: Arc<RwLock<Config>>,
    pub skills: Arc<crate::skills::SkillsRegistry>,
    pub mcps: Arc<crate::mcp::MCPsRegistry>,
    /// Process start time. Surfaced via `daemon/status` `uptimeSecs`.
    pub started_at: Instant,
    /// Resolved unix socket path. Surfaced via `daemon/status`
    /// `socketPath` + `diag/snapshot`.
    pub socket_path: PathBuf,
}

/// One accepted connection. Reads NDJSON, dispatches, writes the
/// response, loops. After `status/subscribe` the same loop multiplexes
/// `status/changed` notifications via `tokio::select!`.
pub async fn handle_connection(stream: UnixStream, state: RpcState) {
    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();

    // Set by `status/subscribe`; its presence also gates the second-
    // subscribe rejection via `HandlerCtx::already_subscribed`.
    let mut status_rx: Option<Box<tokio::sync::broadcast::Receiver<crate::rpc::protocol::StatusResult>>> = None;

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

                let dispatch_result = dispatch(
                    &line,
                    DispatchInput {
                        app: Some(&state.app),
                        status: &state.status,
                        dispatcher: &state.dispatcher,
                        adapter: state.adapter.clone(),
                        config: Some(state.config.clone()),
                        skills: Some(state.skills.clone()),
                        mcps: Some(state.mcps.clone()),
                        connection_already_subscribed: status_rx.is_some(),
                        started_at: Some(state.started_at),
                        socket_path: Some(state.socket_path.as_path()),
                    },
                ).await;

                let DispatchOutput { response, new_status_rx } = dispatch_result;

                if let Some(rx) = new_status_rx {
                    status_rx = Some(rx);
                }

                // Shutdown signal rides in the response payload as
                // `{"killed": true}` (`daemon/kill`) or `{"exiting": true}`
                // (`daemon/shutdown`); captured before `response` moves so
                // we can act after the flush.
                let kill_signalled = matches!(
                    &response.outcome,
                    Outcome::Success { result }
                        if result.get("killed").and_then(|v| v.as_bool()) == Some(true)
                            || result.get("exiting").and_then(|v| v.as_bool()) == Some(true)
                );

                if !write_line(&mut writer, &response, "response").await {
                    return;
                }

                if kill_signalled {
                    crate::daemon::shutdown(&state.app, state.adapter.as_ref()).await;
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
                        if !write_line(&mut writer, &notif, "status/changed").await {
                            return;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!(n, "rpc: subscriber lagged, re-pushing current snapshot");
                        // The select! loop waits for the *next* transition;
                        // if nothing further happens, the subscriber sits on
                        // stale state indefinitely. Re-send the current
                        // snapshot so the peer resynchronises immediately.
                        let notif = StatusChangedNotification::new(state.status.get());
                        if !write_line(&mut writer, &notif, "status/changed (resync)").await {
                            return;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        // Broadcast sender dropped — nothing left to receive. Close.
                        return;
                    }
                }
            }
        }
    }
}

/// Serialize `value` as a single NDJSON line and flush it on the
/// connection's writer. Returns `false` when the connection should
/// close (write or serialize error already logged); `true` otherwise.
/// Centralises the four serialize-newline-write-flush sites in
/// `handle_connection`.
async fn write_line(writer: &mut tokio::net::unix::OwnedWriteHalf, value: &impl serde::Serialize, ctx: &str) -> bool {
    let mut bytes = match serde_json::to_vec(value) {
        Ok(b) => b,
        Err(err) => {
            error!(%err, %ctx, "rpc: failed to serialize");
            return false;
        }
    };
    bytes.push(b'\n');
    if let Err(err) = writer.write_all(&bytes).await {
        warn!(%err, %ctx, "rpc: write error, closing connection");
        return false;
    }
    let _ = writer.flush().await;
    true
}

/// Dispatch result. Single struct so the connection loop can grow new
/// fields without reflowing every match arm.
struct DispatchOutput {
    response: Response,
    new_status_rx: Option<Box<tokio::sync::broadcast::Receiver<crate::rpc::protocol::StatusResult>>>,
}

/// Per-connection state the dispatcher reads. Bundles every shared
/// reference so `dispatch` is a two-arg function: this state + the
/// raw NDJSON line.
pub(crate) struct DispatchInput<'a> {
    pub app: Option<&'a tauri::AppHandle>,
    pub status: &'a StatusBroadcast,
    pub dispatcher: &'a RpcDispatcher,
    pub adapter: Arc<dyn Adapter>,
    pub config: Option<Arc<RwLock<Config>>>,
    pub skills: Option<Arc<crate::skills::SkillsRegistry>>,
    pub mcps: Option<Arc<crate::mcp::MCPsRegistry>>,
    pub connection_already_subscribed: bool,
    pub started_at: Option<Instant>,
    pub socket_path: Option<&'a std::path::Path>,
}

/// Parse one NDJSON line → `RpcDispatcher` → JSON-RPC `Response`.
/// Tests pass `None` for `input.app` + a stand-alone fixture so they
/// can drive routing without a live Tauri runtime.
async fn dispatch(line: &str, input: DispatchInput<'_>) -> DispatchOutput {
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
        app: input.app,
        status: input.status,
        adapter: input.adapter,
        config: input.config,
        skills: input.skills,
        mcps: input.mcps,
        already_subscribed: input.connection_already_subscribed,
        started_at: input.started_at,
        socket_path: input.socket_path,
    };

    let outcome = input.dispatcher.dispatch(&method, params, ctx).await;

    match outcome {
        Ok(HandlerOutcome::Reply(value)) => DispatchOutput {
            response: Response::success(Some(id), value),
            new_status_rx: None,
        },
        Ok(HandlerOutcome::StatusSubscribed(snapshot, rx)) => DispatchOutput {
            response: Response::success(Some(id), snapshot),
            new_status_rx: Some(rx),
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
    use crate::adapters::AcpAdapter;
    use crate::config::Config;

    /// Driver for envelope / framing tests: no `AppHandle`, a fresh
    /// `StatusBroadcast`, a throwaway dispatcher + adapter per call.
    /// Handlers that need the app surface `-32603`; everything else
    /// runs through to the handler's real logic.
    async fn run(line: &str) -> serde_json::Value {
        let status = StatusBroadcast::new(true);
        let dispatcher = RpcDispatcher::with_defaults();
        let config = Arc::new(RwLock::new(Config::default()));
        let acp = Arc::new(AcpAdapter::new(Config::default(), Arc::new(StatusBroadcast::new(true))));
        let adapter: Arc<dyn Adapter> = acp.clone();
        let out = dispatch(
            line,
            DispatchInput {
                app: None,
                status: &status,
                dispatcher: &dispatcher,
                adapter,
                config: Some(config),
                skills: None,
                mcps: None,
                connection_already_subscribed: false,
                started_at: None,
                socket_path: None,
            },
        )
        .await;
        serde_json::to_value(out.response).unwrap()
    }

    #[tokio::test]
    async fn daemon_kill_returns_killed_true() {
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
        let out = run(r#"{"id":1,"method":"overlay/toggle"}"#).await;
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

    /// Bare `"status"` (no `/verb`) falls through to `-32601 method_not_found`.
    #[tokio::test]
    async fn bare_status_method_name_is_method_not_found() {
        let out = run(r#"{"jsonrpc":"2.0","id":9,"method":"status"}"#).await;
        assert_eq!(out["error"]["code"], -32601, "{out}");

        let out = run(r#"{"jsonrpc":"2.0","id":10,"method":"subscribe"}"#).await;
        assert_eq!(out["error"]["code"], -32601, "{out}");
    }

    #[tokio::test]
    async fn invalid_request_wrong_jsonrpc_version() {
        let out = run(r#"{"jsonrpc":"1.0","id":11,"method":"overlay/toggle"}"#).await;
        assert_eq!(out["error"]["code"], -32600);
        assert_eq!(out["id"], 11);
    }

    /// `overlay/toggle` with no `params` key at all deserializes
    /// cleanly. Handler surfaces `-32603` because `app` is None in the
    /// test harness, but the absence of `-32600` / `-32602` proves the
    /// line parsed + routed correctly.
    #[tokio::test]
    async fn overlay_toggle_without_params_is_handled() {
        let out = run(r#"{"jsonrpc":"2.0","id":12,"method":"overlay/toggle"}"#).await;
        assert_eq!(out["error"]["code"], -32603);
    }

    /// Every bare legacy method name surfaces `-32601` after K-239's rename.
    #[tokio::test]
    async fn bare_legacy_method_names_are_method_not_found() {
        for method in ["submit", "cancel", "toggle", "kill", "session-info"] {
            let body = format!(r#"{{"jsonrpc":"2.0","id":99,"method":"{method}"}}"#);
            let out = run(&body).await;
            assert_eq!(out["error"]["code"], -32601, "bare {method} must be -32601: {out}");
        }
    }

    /// Pruned namespaces (`session/*`, `events/*`, `agents/*`,
    /// `commands/*`, `config/*`, `mcps/*`, `models/*`, `modes/*`,
    /// `profiles/*`, `sessions/*`, `skills/*`, `window/*`) all return
    /// `-32601`. Webview consumers go through Tauri commands;
    /// hyprland-bind users use `overlay/toggle` instead of
    /// `window/toggle`.
    #[tokio::test]
    async fn pruned_namespaces_return_method_not_found() {
        for method in [
            "session/submit",
            "session/cancel",
            "events/subscribe",
            "agents/list",
            "config/profiles",
            "mcps/list",
            "models/list",
            "modes/list",
            "profiles/list",
            "sessions/list",
            "skills/list",
            "window/toggle",
        ] {
            let body = format!(r#"{{"jsonrpc":"2.0","id":1,"method":"{method}"}}"#);
            let out = run(&body).await;
            assert_eq!(out["error"]["code"], -32601, "pruned {method} must be -32601: {out}");
        }
    }

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
    async fn overlay_toggle_without_app_is_internal_error() {
        let out = run(r#"{"jsonrpc":"2.0","id":5,"method":"overlay/toggle"}"#).await;
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
                    DispatchInput {
                        app: None,
                        status: &status,
                        dispatcher: &dispatcher,
                        adapter: adapter.clone(),
                        config: Some(config.clone()),
                        skills: None,
                        mcps: None,
                        connection_already_subscribed: false,
                        started_at: None,
                        socket_path: None,
                    },
                )
                .await;
                let mut bytes = serde_json::to_vec(&out.response).unwrap();
                bytes.push(b'\n');
                writer.write_all(&bytes).await.unwrap();
                writer.flush().await.unwrap();
            }
        });

        client
            .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"status/get\"}\n")
            .await
            .unwrap();

        let mut buf = String::new();
        BufReader::new(&mut client).read_line(&mut buf).await.unwrap();

        let v: serde_json::Value = serde_json::from_str(&buf).unwrap();
        assert_eq!(v["id"], 1);
        assert_eq!(v["result"]["state"], "idle");

        drop(client);
        task.await.unwrap();
    }

    /// A second `status/subscribe` on the same connection must return
    /// `-32600 invalid_request`.
    #[tokio::test]
    async fn double_subscribe_on_same_connection_is_rejected() {
        let broadcast = StatusBroadcast::new(true);
        let dispatcher = RpcDispatcher::with_defaults();
        let config = Arc::new(RwLock::new(Config::default()));
        let acp = Arc::new(AcpAdapter::new(Config::default(), Arc::new(StatusBroadcast::new(true))));
        let adapter: Arc<dyn Adapter> = acp.clone();

        let _ = acp;
        let out = dispatch(
            r#"{"jsonrpc":"2.0","id":1,"method":"status/subscribe"}"#,
            DispatchInput {
                app: None,
                status: &broadcast,
                dispatcher: &dispatcher,
                adapter: adapter.clone(),
                config: Some(config.clone()),
                skills: None,
                mcps: None,
                connection_already_subscribed: false,
                started_at: None,
                socket_path: None,
            },
        )
        .await;
        let v = serde_json::to_value(&out.response).unwrap();
        assert_eq!(v["result"]["state"], "idle", "first subscribe succeeds");
        assert!(out.new_status_rx.is_some(), "first subscribe returns a receiver");

        let out = dispatch(
            r#"{"jsonrpc":"2.0","id":2,"method":"status/subscribe"}"#,
            DispatchInput {
                app: None,
                status: &broadcast,
                dispatcher: &dispatcher,
                adapter,
                config: Some(config),
                skills: None,
                mcps: None,
                connection_already_subscribed: true,
                started_at: None,
                socket_path: None,
            },
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

    /// When a subscriber falls behind capacity, `recv()` returns
    /// `RecvError::Lagged(n)`. The server re-pushes `status.get()` so
    /// the subscriber resynchronises immediately.
    #[tokio::test]
    async fn lagged_subscriber_receives_resynced_snapshot() {
        use crate::rpc::protocol::{AgentState, StatusResult};

        let broadcast = Arc::new(StatusBroadcast::new(true));
        let (_snap_initial, mut rx) = broadcast.subscribe();

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

        let final_state = StatusResult {
            state: AgentState::Awaiting,
            visible: false,
            active_session: Some("sess-resync".into()),
        };
        broadcast.set(final_state.clone());

        let first = rx.recv().await;
        assert!(
            matches!(first, Err(tokio::sync::broadcast::error::RecvError::Lagged(_))),
            "expected Lagged, got {first:?}"
        );

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

    /// Integration test: subscribe through `dispatch`, flip state via
    /// the shared `StatusBroadcast`, assert the subscriber receives
    /// exactly one `status/changed` notification, and the connection
    /// cleans up without leaking senders.
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
            let mut status_rx: Option<Box<tokio::sync::broadcast::Receiver<StatusResult>>> = None;

            loop {
                tokio::select! {
                    line_result = lines.next_line() => {
                        match line_result {
                            Ok(Some(l)) if l.trim().is_empty() => continue,
                            Ok(Some(l)) => {
                                let _ = &acp;
                                let out = dispatch(
                                    &l,
                                    DispatchInput {
                                        app: None,
                                        status: &broadcast_clone,
                                        dispatcher: &dispatcher,
                                        adapter: adapter.clone(),
                                        config: Some(config.clone()),
                                        skills: None,
                                        mcps: None,
                                        connection_already_subscribed: status_rx.is_some(),
                                        started_at: None,
                                        socket_path: None,
                                    },
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
}
