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

use crate::adapters::InstanceEvent;
use crate::remote::pair::PAIR_EXPIRY;
use crate::remote::server::RemoteState;

/// Per-WS task. Pair-on-connect → authenticated proxy.
pub async fn handle_socket(socket: WebSocket, state: RemoteState, peer: SocketAddr) {
    let (mut sink, mut stream) = socket.split();

    // Mint pending pair, push the welcome frame, await captain
    // confirm. `confirm_rx` fires on `pair_store::confirm()` from
    // the desktop side (Tauri command).
    let (pending_id, code, confirm_rx) = state.pairs.create(peer.to_string());
    let qr_payload = code.as_str().to_string();
    let pending_frame = json!({
        "type": "pending",
        "pendingId": pending_id.to_string(),
        "code": code.as_str(),
        "qrPayload": qr_payload,
        "expiresInSeconds": PAIR_EXPIRY.as_secs(),
    });

    if !send_text(&mut sink, &pending_frame.to_string()).await {
        return;
    }

    // Emit the desktop pair-request event. Captain's overlay listens
    // and pops the confirm modal.
    let _ = state.app.emit(
        "remote:pair-request",
        json!({
            "pendingId": pending_id.to_string(),
            "code": code.as_str(),
            "remoteAddr": peer.to_string(),
        }),
    );

    // Race: captain confirms (oneshot fires) vs expiry vs phone
    // closes the WS first.
    let confirmed = tokio::select! {
        biased;
        signal = confirm_rx => signal.is_ok(),
        _ = tokio::time::sleep(PAIR_EXPIRY) => false,
        // Drain any phone-side messages during the pending window;
        // none are expected, but a clean disconnect must propagate.
        _ = drain_during_pending(&mut stream) => false,
    };

    if !confirmed {
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

    // Tell the phone we're authenticated; stream now carries RPC.
    if !send_text(&mut sink, &json!({ "type": "authenticated" }).to_string()).await {
        return;
    }
    tracing::info!(%peer, %pending_id, "remote: WS authenticated");

    // Subscribe to the InstanceEvent broadcast for push notifications.
    // The Tauri event bridge already has its own subscriber; broadcast
    // channels fan out to every receiver, so we just spin up another.
    let mut events_rx = subscribe_events(&state);

    // Proxy loop: client text frame → RpcDispatcher → response
    // text frame. Events from the InstanceEvent broadcast also push
    // out as `{ type: "event", name, payload }` frames.
    let mut status_rx: Option<Box<broadcast::Receiver<crate::rpc::protocol::StatusResult>>> = None;

    loop {
        tokio::select! {
            // ── Client message ──────────────────────────────────
            msg = stream.next() => {
                let frame = match msg {
                    Some(Ok(m)) => m,
                    Some(Err(err)) => {
                        tracing::warn!(%peer, %err, "remote: WS read error");
                        return;
                    }
                    None => return,
                };
                match frame {
                    Message::Text(text) => {
                        let line = text.to_string();
                        let DispatchResult { response_text, new_status_rx } =
                            dispatch_line(&line, &state, status_rx.is_some()).await;
                        if let Some(rx) = new_status_rx {
                            status_rx = Some(rx);
                        }
                        if let Some(text) = response_text {
                            if !send_text(&mut sink, &text).await {
                                return;
                            }
                        }
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
                        return;
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
                            return;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(%peer, n, "remote: WS event subscriber lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
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
                            return;
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_))
                    | Err(broadcast::error::RecvError::Closed) => {}
                }
            }
        }
    }
}

struct DispatchResult {
    response_text: Option<String>,
    new_status_rx: Option<Box<broadcast::Receiver<crate::rpc::protocol::StatusResult>>>,
}

/// Dispatch one NDJSON line through `RpcDispatcher`. Reuses the same
/// `dispatch_line` entry point the unix socket calls.
async fn dispatch_line(line: &str, state: &RemoteState, already_subscribed: bool) -> DispatchResult {
    let result = crate::rpc::server::dispatch_line(
        line,
        crate::rpc::server::DispatchInput {
            app: Some(&state.app),
            status: &state.status,
            dispatcher: &state.dispatcher,
            adapter: state.adapter.clone(),
            config: Some(state.config.clone()),
            skills: Some(state.skills.clone()),
            mcps: Some(state.mcps.clone()),
            connection_already_subscribed: already_subscribed,
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

/// During pending, drain client frames so the underlying tcp socket
/// detects close cleanly. Phone usually sends nothing during the
/// pending window — this exists to convert "phone closed" into the
/// `confirm_rx` race-loser path.
async fn drain_during_pending(stream: &mut futures_util::stream::SplitStream<WebSocket>) {
    while let Some(msg) = stream.next().await {
        match msg {
            Ok(Message::Close(_)) | Err(_) => return,
            _ => continue,
        }
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

/// Map an `InstanceEvent` variant onto the same Tauri-event name the
/// embedded WebView already listens on. UI consumers don't have to
/// branch on transport — same event names everywhere.
fn event_envelope(evt: &InstanceEvent) -> (&'static str, serde_json::Value) {
    let name = match evt {
        InstanceEvent::State { .. } => "acp:instance-state",
        InstanceEvent::Transcript { .. } => "acp:transcript",
        InstanceEvent::PermissionRequest { .. } => "acp:permission-request",
        InstanceEvent::TurnStarted { .. } => "acp:turn-started",
        InstanceEvent::TurnEnded { .. } => "acp:turn-ended",
        InstanceEvent::InstancesChanged { .. } => "acp:instances-changed",
        InstanceEvent::InstancesFocused { .. } => "acp:instances-focused",
        InstanceEvent::InstanceRenamed { .. } => "acp:instance-renamed",
        InstanceEvent::Terminal { .. } => "acp:terminal",
        InstanceEvent::DaemonReloaded { .. } => "daemon:reloaded",
        InstanceEvent::SessionInfoUpdate { .. } => "acp:session-info-update",
        InstanceEvent::CurrentModeUpdate { .. } => "acp:current-mode-update",
        InstanceEvent::UsageUpdate { .. } => "acp:usage-update",
        InstanceEvent::ConfigOptionsUpdate { .. } => "acp:config-options-update",
        InstanceEvent::InstanceMeta { .. } => "acp:instance-meta",
        InstanceEvent::SystemPromptInjected { .. } => "acp:system-prompt-injected",
    };
    let payload = serde_json::to_value(evt).unwrap_or(serde_json::Value::Null);
    (name, payload)
}
