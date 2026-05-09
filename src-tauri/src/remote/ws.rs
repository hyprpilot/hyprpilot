//! WS handler — pair-on-connect, then proxy NDJSON-shaped JSON-RPC
//! between the WS frames and `RpcDispatcher`. Same dispatcher the
//! unix socket calls; same JSON-RPC envelopes on the wire.
//!
//! Lifecycle per connection:
//! 1. Upgrade lands. Pending pair is minted; daemon emits a
//!    `remote:pair-request` Tauri event for the desktop overlay.
//! 2. The daemon sends an unauthenticated `{ "type": "pending",
//!    "pendingId", "code", "qrPayload" }` frame so the phone can
//!    show the captain what to read aloud / scan against.
//! 3. WS task awaits the `oneshot::Receiver<()>` from the pair
//!    store. Captain confirms on desktop → signal fires →
//!    daemon sends `{ "type": "authenticated" }` and enters the
//!    RPC proxy loop.
//! 4. Proxy loop: client text frame is treated as a single NDJSON
//!    JSON-RPC request line; response is sent back as a single
//!    text frame. `InstanceEvent` broadcast fans out as
//!    `{ "type": "event", "name": "acp:transcript", "payload": ... }`
//!    frames.

use std::net::SocketAddr;

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tauri::Emitter;
use tokio::sync::broadcast;
use tokio::task::JoinSet;
use uuid::Uuid;

use crate::adapters::InstanceEvent;
use crate::remote::pair::{ConfirmSide, CreatedPair, PairStore, PAIR_EXPIRY};
use crate::remote::server::RemoteState;
use crate::remote::session::SessionTokens;

/// Per-connection cap on queued dispatch responses. Backpressure
/// kicks in once the writer is more than this many frames behind —
/// the spawn site `await`s on send, naturally serializing inflight
/// dispatches against a peer that's blasting frames but not
/// reading. 64 covers every realistic concurrent-invoke fanout
/// (boot snapshot is one; brim-sync prefetches a handful) without
/// letting a degenerate peer eat unbounded memory.
const DISPATCH_OUTBOUND_CAPACITY: usize = 64;

/// Per-WS task. Pair-on-connect → authenticated proxy.
pub async fn handle_socket(socket: WebSocket, state: RemoteState, peer: SocketAddr) {
    let (mut sink, mut stream) = socket.split();

    // Mint pending pair (two distinct codes, one per side), push
    // the welcome frame, await captain confirm. `confirm_rx` fires
    // on `pair_store::confirm()` from EITHER the desktop side (Tauri
    // command, presenting the device's code) OR the connecting
    // device side (`{type:"confirm", code}` WS frame, presenting the
    // desktop's code — typically obtained by the device camera
    // scanning the desktop modal's QR).
    let CreatedPair {
        pending_id,
        device_code,
        desktop_code,
        confirm_rx: mut confirm_rx_owned,
    } = state.pairs.create(peer.to_string());

    // The connecting device receives BOTH codes:
    //   - `deviceCode` is what it should display (its own identity).
    //   - `desktopCode` is what it must obtain (by scanning) to
    //     authenticate from this side.
    let pending_frame = json!({
        "type": "pending",
        "pendingId": pending_id.to_string(),
        "deviceCode": device_code.as_str(),
        "desktopCode": desktop_code.as_str(),
        "expiresInSeconds": PAIR_EXPIRY.as_secs(),
    });

    if !send_text(&mut sink, &pending_frame.to_string()).await {
        return;
    }

    // Emit the desktop pair-request event. Captain's overlay listens
    // and pops the confirm modal. Mirrors the WS payload — the
    // desktop renders `desktopCode` as its own QR + words, expects
    // `deviceCode` as input.
    let _ = state.app.emit(
        "remote:pair-request",
        json!({
            "pendingId": pending_id.to_string(),
            "deviceCode": device_code.as_str(),
            "desktopCode": desktop_code.as_str(),
            "remoteAddr": peer.to_string(),
        }),
    );

    // Race during the pending window:
    // - `confirm_rx` fires (desktop confirmed via Tauri command, OR
    //   the phone self-confirmed via a WS `{type:"confirm"}` frame
    //   from scanning the desktop's QR — both code paths land at
    //   `PairStore::confirm` which fires the same oneshot)
    // - 60s expiry tick
    // - phone closes / errors
    // - phone sends a `{type:"confirm", code}` frame → process it
    //   in-loop; the resulting signal lands on `confirm_rx` on the
    //   next iteration
    let mut expire = Box::pin(tokio::time::sleep(PAIR_EXPIRY));
    let confirmed = loop {
        tokio::select! {
            biased;
            signal = &mut confirm_rx_owned => break signal.is_ok(),
            _ = &mut expire => break false,
            msg = stream.next() => {
                match msg {
                    None => break false,
                    Some(Err(err)) => {
                        tracing::warn!(%peer, %err, "remote: WS read error during pending");
                        break false;
                    }
                    Some(Ok(Message::Close(_))) => break false,
                    Some(Ok(Message::Text(text))) => {
                        let outcome = handle_pending_text(
                            text.as_str(),
                            &state.pairs,
                            &state.sessions,
                            &pending_id,
                        );
                        if let Some(rejection) = outcome {
                            let _ = send_text(
                                &mut sink,
                                &json!({
                                    "type": rejection.frame_type,
                                    "reason": rejection.reason,
                                })
                                .to_string(),
                            )
                            .await;
                        }
                    }
                    _ => {} // ping/pong/binary — ignore until pair completes
                }
            }
        }
    };

    if !confirmed {
        // Tell the desktop modal to close — the pending it was
        // showing is dead. Outcome `rejected` covers timeout,
        // captain-reject, attempt-cap, and connection drop alike;
        // the modal flips out the same way for all of them.
        let _ = state.app.emit(
            "remote:pair-resolved",
            json!({
                "pendingId": pending_id.to_string(),
                "outcome": "rejected",
            }),
        );
        let _ = sink
            .send(Message::Text(
                json!({ "type": "rejected", "reason": "pair not confirmed" })
                    .to_string()
                    .into(),
            ))
            .await;
        // Clean up pending state if it's still hanging around.
        state.pairs.reject(&pending_id);
        return;
    }

    // Pair confirmed — close the desktop modal regardless of which
    // side committed. Without this signal the modal would stay open
    // forever when the device side authenticates first (the captain
    // scanned the desktop's QR with the phone). Ride the same Tauri
    // event the modal already listens for new requests on.
    let _ = state.app.emit(
        "remote:pair-resolved",
        json!({
            "pendingId": pending_id.to_string(),
            "outcome": "confirmed",
        }),
    );

    // Mint a session token bound to this peer's address. The device
    // stores it in localStorage and presents it on the next reconnect
    // via a `{type:"hello"}` frame, skipping the captain-confirm
    // dance entirely. Survives page reloads but not daemon restart —
    // mirrors the rest of the bridge's "no disk persistence" model.
    let session_token = state.sessions.mint(peer.to_string());

    // Tell the phone we're authenticated and hand off the session
    // token in the same frame.
    if !send_text(
        &mut sink,
        &json!({ "type": "authenticated", "sessionToken": session_token }).to_string(),
    )
    .await
    {
        return;
    }
    tracing::info!(%peer, %pending_id, "remote: WS authenticated");

    // Subscribe to the InstanceEvent broadcast for push notifications.
    // The Tauri event bridge already has its own subscriber; broadcast
    // channels fan out to every receiver, so we just spin up another.
    let mut events_rx = subscribe_events(&state);

    // A handful of events are emitted via direct `app.emit(...)` calls
    // outside the InstanceEvent broadcast (e.g. `composer:draft-append`
    // from the prompts handler). Subscribe to each by name through
    // Tauri's `Listener` API and fan them into a shared mpsc the
    // select loop drains. The guard releases the listener IDs on
    // drop — no manual unlisten on every return path.
    let (direct_relay_tx, mut direct_relay_rx) = tokio::sync::mpsc::unbounded_channel::<(String, serde_json::Value)>();
    let _direct_relay_guard = DirectRelayGuard::new(&state.app, direct_relay_tx);

    // Proxy loop: client text frames → RpcDispatcher → response
    // text frames. Events from the InstanceEvent broadcast also push
    // out as `{ type: "event", name, payload }` frames.
    //
    // **Each request is dispatched on its own tokio task** so a slow
    // handler (`session_list` spawning an ephemeral agent via bunx
    // takes ~430ms; future handlers may block longer) can't stall the
    // WS read loop. Without this, every other invoke from the UI's
    // boot path queues at the OS socket buffer behind the slow one
    // — captain sees the loading screen freeze at "configuring window
    // / reading $HOME" while the daemon was actually responding to
    // `session_list` first. Concurrent dispatch keeps the inbound
    // pipe drained; responses funnel back through the
    // `outbound_rx` mpsc so per-WS write ordering stays sequential
    // (one tokio task owns the sink, never two writers racing).
    let mut status_rx: Option<Box<broadcast::Receiver<crate::rpc::protocol::StatusResult>>> = None;
    // Bounded mpsc + per-connection `JoinSet` for the spawned
    // dispatch tasks. Bounded so a peer that floods text frames but
    // stops reading can't OOM the daemon (each pending response sits
    // in `outbound_rx` until the writer drains it; backpressure
    // serializes the spawn site naturally). The JoinSet so we abort
    // every in-flight dispatch when the connection drops, instead of
    // leaving handlers like `session_list` (spawns + tears down a
    // bunx agent over ~430ms) running against a dropped peer holding
    // an `Arc<dyn Adapter>` clone.
    let (outbound_tx, mut outbound_rx) = tokio::sync::mpsc::channel::<DispatchOutbound>(DISPATCH_OUTBOUND_CAPACITY);
    let mut dispatchers: JoinSet<()> = JoinSet::new();

    'proxy: loop {
        tokio::select! {
            // `biased;` polls branches in declaration order so dispatch
            // responses always drain before events (and pings before
            // either). Without this, an instance flooding the broadcast
            // (transcript chunks during a streaming reply, or every
            // `session_list` spawning a short-lived agent) randomly
            // starves the outbound drain — boot-path invokes like
            // `get_home_dir` get queued in `outbound_rx` forever while
            // the select! keeps picking `events_rx.recv()`. The
            // captain sees the loading screen freeze on whatever
            // step's awaiting.
            biased;

            // ── Outbound drain (dispatch responses, status_rx swap) ─
            outbound = outbound_rx.recv() => {
                match outbound {
                    Some(DispatchOutbound::Response(text)) => {
                        if !send_text(&mut sink, &text).await {
                            break 'proxy;
                        }
                    }
                    Some(DispatchOutbound::StatusRx(rx)) => {
                        status_rx = Some(rx);
                    }
                    None => break 'proxy,
                }
            }

            // ── Client message ──────────────────────────────────
            msg = stream.next() => {
                let frame = match msg {
                    Some(Ok(m)) => m,
                    Some(Err(err)) => {
                        tracing::warn!(%peer, %err, "remote: WS read error");
                        break 'proxy;
                    }
                    None => break 'proxy,
                };
                match frame {
                    Message::Text(text) => {
                        let line = text.to_string();
                        // `StatusBroadcast::subscribe` is atomic against
                        // `set` (snapshot mutex), so a `status/subscribe`
                        // request that lands AFTER a `set` still
                        // observes the post-set snapshot AND every
                        // subsequent `set` on its receiver — no missed
                        // notifications. So we capture
                        // `already_subscribed` here, spawn, and let the
                        // handler do the subscribe.
                        let already_subscribed = status_rx.is_some();
                        let state_clone = state.clone();
                        let tx = outbound_tx.clone();
                        dispatchers.spawn(async move {
                            let DispatchResult { response_text, new_status_rx } =
                                dispatch_line(&line, &state_clone, already_subscribed).await;
                            if let Some(rx) = new_status_rx {
                                let _ = tx.send(DispatchOutbound::StatusRx(rx)).await;
                            }
                            if let Some(text) = response_text {
                                let _ = tx.send(DispatchOutbound::Response(text)).await;
                            }
                        });
                    }
                    Message::Binary(_) => {
                        let _ = sink
                            .send(Message::Text(
                                json!({ "type": "error", "message": "binary frames not supported" })
                                    .to_string()
                                    .into(),
                            ))
                            .await;
                    }
                    Message::Ping(p) => {
                        let _ = sink.send(Message::Pong(p)).await;
                    }
                    Message::Close(_) | Message::Pong(_) => {
                        break 'proxy;
                    }
                }
            }

            // ── Push: instance event ────────────────────────────
            evt = events_rx.recv() => {
                match evt {
                    Ok(evt) => {
                        let (name, payload) = event_envelope(&evt);
                        let frame = json!({
                            "type": "event",
                            "name": name,
                            "payload": payload,
                        });
                        if !send_text(&mut sink, &frame.to_string()).await {
                            break 'proxy;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(%peer, n, "remote: WS event subscriber lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => break 'proxy,
                }
            }

            // ── Push: direct Tauri-emit relay ────────────────────
            // (composer:draft-append etc. — events emitted via
            // app.emit() outside the InstanceEvent broadcast)
            relayed = direct_relay_rx.recv() => {
                if let Some((name, payload)) = relayed {
                    let frame = json!({
                        "type": "event",
                        "name": name,
                        "payload": payload,
                    });
                    if !send_text(&mut sink, &frame.to_string()).await {
                        break 'proxy;
                    }
                }
            }

            // ── Push: status/changed (subscriber-only) ──────────
            notif = async {
                match &mut status_rx {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => {
                match notif {
                    Ok(sr) => {
                        let frame = serde_json::to_string(
                            &crate::rpc::protocol::StatusChangedNotification::new(sr),
                        )
                        .unwrap_or_else(|_| "{}".to_string());
                        if !send_text(&mut sink, &frame).await {
                            break 'proxy;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_))
                    | Err(broadcast::error::RecvError::Closed) => {}
                }
            }
        }
    }

    // Connection closed — abort every in-flight dispatch task. Without
    // this, a long-running handler (e.g. `session_list` spawning a
    // bunx agent over ~430ms) keeps running against a dropped peer,
    // holding `Arc<dyn Adapter>` clones until it returns.
    dispatchers.shutdown().await;
}

struct DispatchResult {
    response_text: Option<String>,
    new_status_rx: Option<Box<broadcast::Receiver<crate::rpc::protocol::StatusResult>>>,
}

/// Concurrent-dispatch back-channel. Each spawned dispatch task feeds
/// its response (and any `status/subscribe`-minted receiver) into the
/// per-connection mpsc so the WS write loop can drain in arrival
/// order with a single owner of the sink.
enum DispatchOutbound {
    Response(String),
    StatusRx(Box<broadcast::Receiver<crate::rpc::protocol::StatusResult>>),
}

/// Dispatch one NDJSON line through `RpcDispatcher`. Reuses the same
/// `dispatch_line` entry point the unix socket calls.
///
/// `events/subscribe` lands here as a regular handler outcome; the WS
/// bridge already streams `InstanceEvent`s via its dedicated
/// `events_rx` select arm (subscribed at handshake time, not per-call),
/// so we discard the receiver from the outcome and only ack the
/// reply. The unix-socket transport is the one that actually pins
/// the receiver onto its connection state.
async fn dispatch_line(line: &str, state: &RemoteState, already_subscribed: bool) -> DispatchResult {
    let result = crate::rpc::server::dispatch_line(
        line,
        crate::rpc::server::DispatchInput {
            app: Some(&state.app),
            status: &state.status,
            dispatcher: &state.dispatcher,
            adapter: state.adapter.clone(),
            config: Some(state.config.clone()),
            mcps: Some(state.mcps.clone()),
            connection_already_subscribed: already_subscribed,
            // WS bridge subscribes to events at handshake time, not
            // per-call; treating every connection as "already
            // subscribed" makes a peer-issued `events/subscribe`
            // return `-32600` rather than minting a duplicate
            // receiver the loop wouldn't read from.
            connection_already_events_subscribed: true,
            started_at: Some(state.started_at),
            socket_path: None,
        },
    )
    .await;

    let response_text = serde_json::to_string(&result.response).ok();
    DispatchResult {
        response_text,
        new_status_rx: result.new_status_rx,
    }
}

/// Rejection feedback the WS bridge sends back during the pending
/// phase. `frame_type` is the wire `type` discriminator the device
/// switches on; `reason` is the human-readable explanation.
struct PendingRejection {
    frame_type: &'static str,
    reason: String,
}

/// Process a single text frame received during the pending window.
///
/// Two actionable shapes:
///   - `{type:"confirm", code:"<words>"}` — connecting device sends
///     this after scanning the desktop's QR. The candidate must
///     match the **desktop's** code.
///   - `{type:"hello", sessionToken:"<uuid>"}` — connecting device
///     presenting a token from a prior pair. Skips the captain-
///     confirm dance entirely on validate.
///
/// Anything else is silently ignored (forward-compat).
///
/// Returns `Some(PendingRejection)` for negative feedback the device
/// should react to:
///   - `confirm-rejected` — typed/scanned code didn't match.
///   - `hello-rejected` — session token is unknown (daemon restarted
///     since the token was minted, or the token was forged). Tells
///     the device to clear the cached token; the pair screen is
///     already visible (rendered on `pending`), so the user can
///     fall back to manual pair without action.
///
/// `None` on success or non-actionable frames.
fn handle_pending_text(
    text: &str,
    pairs: &PairStore,
    sessions: &SessionTokens,
    pending_id: &Uuid,
) -> Option<PendingRejection> {
    let parsed: serde_json::Value = serde_json::from_str(text).ok()?;
    let frame_type = parsed.get("type").and_then(|v| v.as_str())?;

    match frame_type {
        "confirm" => {
            let code = parsed.get("code").and_then(|v| v.as_str()).unwrap_or("");
            match pairs.confirm(pending_id, code, ConfirmSide::Device) {
                Ok(()) => None,
                Err(err) => Some(PendingRejection {
                    frame_type: "confirm-rejected",
                    reason: err.to_string(),
                }),
            }
        }
        "hello" => {
            let token = parsed.get("sessionToken").and_then(|v| v.as_str()).unwrap_or("");

            if !sessions.validate(token) {
                tracing::info!(
                    token_len = token.len(),
                    "remote: hello with unknown token — telling device to clear it"
                );
                return Some(PendingRejection {
                    frame_type: "hello-rejected",
                    reason: "unknown session token — pair manually".to_string(),
                });
            }
            // Token validated → fire the same oneshot a normal
            // confirm fires. The remote address is proof of presence
            // here (live WS bound to a known token); the captain
            // already authorised this device once during this daemon
            // run.
            pairs.fast_confirm(pending_id);
            tracing::info!("remote: hello validated — auto-confirming pending pair");
            None
        }
        _ => None,
    }
}

async fn send_text(sink: &mut futures_util::stream::SplitSink<WebSocket, Message>, text: &str) -> bool {
    if let Err(err) = sink.send(Message::Text(text.to_string().into())).await {
        tracing::warn!(%err, "remote: WS send failed");
        return false;
    }
    true
}

fn subscribe_events(state: &RemoteState) -> broadcast::Receiver<InstanceEvent> {
    // The `Adapter` trait exposes `subscribe()` returning a fresh
    // `broadcast::Receiver<InstanceEvent>` keyed off the adapter's
    // underlying `AdapterRegistry`. Same broadcast the Tauri-event
    // bridge subscribes to.
    state.adapter.subscribe()
}

/// Tauri events emitted via direct `app.emit(...)` calls outside the
/// `InstanceEvent` broadcast that should still reach a remote SPA.
/// Each entry is registered as a Tauri global listener per WS task;
/// IDs released on connection drop via `DirectRelayGuard`.
///
/// Currently:
/// - `composer:draft-append` — `ctl prompts send --draft` lands a
///   prompt in the composer without dispatching. Surfaced over the
///   bridge so the same flow works targeting a remote overlay.
///
/// Pair-flow events (`remote:pair-request`, `remote:pair-resolved`)
/// are intentionally NOT relayed — they're desktop-side modal
/// signals, not anything a remote SPA should react to.
const DIRECT_RELAY_EVENTS: &[&str] = &["composer:draft-append"];

/// RAII handle that owns the per-WS Tauri-event listener IDs and
/// releases them on drop. Holds a clone of the AppHandle so it can
/// call `unlisten` without going back through `RemoteState`.
struct DirectRelayGuard {
    app: tauri::AppHandle,
    ids: Vec<u32>,
}

impl DirectRelayGuard {
    fn new(app: &tauri::AppHandle, tx: tokio::sync::mpsc::UnboundedSender<(String, serde_json::Value)>) -> Self {
        use tauri::Listener;
        let ids = DIRECT_RELAY_EVENTS
            .iter()
            .map(|name| {
                let tx = tx.clone();
                let event_name = (*name).to_string();
                app.listen(*name, move |evt| {
                    let payload: serde_json::Value =
                        serde_json::from_str(evt.payload()).unwrap_or(serde_json::Value::Null);
                    let _ = tx.send((event_name.clone(), payload));
                })
            })
            .collect();
        Self { app: app.clone(), ids }
    }
}

impl Drop for DirectRelayGuard {
    fn drop(&mut self) {
        use tauri::Listener;
        for id in self.ids.drain(..) {
            self.app.unlisten(id);
        }
    }
}

/// Map an `InstanceEvent` variant onto the same Tauri-event name the
/// embedded WebView already listens on. UI consumers don't have to
/// branch on transport — same event names everywhere. The name lookup
/// itself lives on `InstanceEvent::event_name()` so the unix-socket
/// `events/subscribe` handler shares the single source of truth.
fn event_envelope(evt: &InstanceEvent) -> (&'static str, serde_json::Value) {
    let name = evt.event_name();
    let payload = serde_json::to_value(evt).unwrap_or(serde_json::Value::Null);
    (name, payload)
}
