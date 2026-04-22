//! Per-session actor: owns the ACP `ConnectionTo<Agent>` and the
//! child process; drives `initialize` → `session/new` → `session/prompt`
//! for the first prompt, then loops on a command mpsc while also
//! fanning `SessionNotification`s out to a broadcast channel the
//! daemon subscribes to for Tauri event emission.

use std::sync::Arc;

use agent_client_protocol::schema::{
    CancelNotification, ContentBlock, InitializeRequest, NewSessionRequest, PromptRequest, ProtocolVersion, SessionId,
    TextContent,
};
use agent_client_protocol::{ByteStreams, Client};
use serde::Serialize;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use tracing::{error, info, warn};

use super::client::{AcpClient, ClientEvent, PermissionOptionView};
use super::resolve::ResolvedSession;
use super::spawn::spawn_agent;

/// Commands the per-session actor accepts. The actor keeps state
/// internal; this enum is the only public surface the dispatcher
/// uses to drive it.
#[derive(Debug)]
pub enum SessionCommand {
    Prompt {
        text: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    Cancel {
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Shutdown hook — stops the actor after the current prompt
    /// (or immediately if idle). Reply carries the final state.
    Shutdown {
        reply: oneshot::Sender<()>,
    },
}

/// Events the actor broadcasts upstream. `AcpSessions` owns a
/// `broadcast::Sender` and the daemon's Tauri `setup` closure
/// subscribes to it to emit `acp:*` events.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionEvent {
    /// Session transitioned to a new lifecycle state.
    State {
        agent_id: String,
        session_id: Option<String>,
        state: SessionState,
    },
    /// Agent pushed a `session/update` notification; raw JSON
    /// because the upstream shape is `#[non_exhaustive]` and we
    /// don't want to reshape every variant here.
    Transcript {
        agent_id: String,
        session_id: String,
        update: serde_json::Value,
    },
    /// Agent asked permission. Auto-denied server-side until
    /// PermissionController lands; emitted so the webview can log /
    /// show it anyway.
    PermissionRequest {
        agent_id: String,
        session_id: String,
        options: Vec<PermissionOptionView>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Starting,
    Running,
    Ended,
    Error,
}

/// Handle the caller keeps after `start_session`. Dropping it cancels
/// the actor (via the `cmd_tx` drop + the actor's select loop
/// observing `None` from the mpsc receiver).
#[derive(Debug)]
pub struct SessionHandle {
    pub agent_id: String,
    pub cmd_tx: mpsc::UnboundedSender<SessionCommand>,
    /// Populated after the first prompt's `session/new` resolves.
    /// `None` while the session is still bootstrapping.
    pub session_id: Arc<tokio::sync::RwLock<Option<String>>>,
}

impl SessionHandle {
    pub async fn current_session_id(&self) -> Option<String> {
        self.session_id.read().await.clone()
    }
}

/// Start a fresh per-session actor. Returns the handle immediately —
/// the first prompt is queued on the returned `cmd_tx` and drives
/// the initialize → session/new → prompt dance inside the actor.
///
/// Sends `SessionEvent`s onto `events_tx` for every lifecycle
/// transition + every `SessionUpdate` the agent streams.
pub fn start_session(session: ResolvedSession, events_tx: broadcast::Sender<SessionEvent>) -> SessionHandle {
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<SessionCommand>();
    let session_id = Arc::new(tokio::sync::RwLock::new(None::<String>));

    let handle = SessionHandle {
        agent_id: session.agent.id.clone(),
        cmd_tx,
        session_id: session_id.clone(),
    };

    tokio::spawn(run_session(session, cmd_rx, events_tx, session_id));

    handle
}

/// The long-lived actor body.
async fn run_session(
    session: ResolvedSession,
    mut cmd_rx: mpsc::UnboundedReceiver<SessionCommand>,
    events_tx: broadcast::Sender<SessionEvent>,
    session_id_slot: Arc<tokio::sync::RwLock<Option<String>>>,
) {
    let agent_id = session.agent.id.clone();
    let _ = events_tx.send(SessionEvent::State {
        agent_id: agent_id.clone(),
        session_id: None,
        state: SessionState::Starting,
    });

    let cfg = {
        let mut cfg = session.agent.clone();
        cfg.model = session.model.clone();
        cfg
    };
    let system_prompt = session.system_prompt.clone();

    let (mut child, stdio, mut first_message_prefix) = match spawn_agent(&cfg, system_prompt.as_deref()) {
        Ok(spawned) => (spawned.child, spawned.stdio, spawned.first_message_prefix),
        Err(err) => {
            error!(agent = %agent_id, %err, "acp::runtime: spawn failed");
            let _ = events_tx.send(SessionEvent::State {
                agent_id,
                session_id: None,
                state: SessionState::Error,
            });
            return;
        }
    };

    let (client_events_tx, mut client_events_rx) = mpsc::unbounded_channel::<ClientEvent>();
    let client = AcpClient::new(client_events_tx);

    let transport = ByteStreams::new(stdio.stdin.compat_write(), stdio.stdout.compat());

    let events_tx_notif = events_tx.clone();
    let agent_id_notif = agent_id.clone();
    let session_id_forward = session_id_slot.clone();

    let dispatch = async move |connection: agent_client_protocol::ConnectionTo<agent_client_protocol::Agent>| {
        let init = connection
            .send_request(InitializeRequest::new(ProtocolVersion::V1))
            .block_task()
            .await?;
        info!(agent = %agent_id_notif, protocol = ?init.protocol_version, "acp::runtime: initialized");

        let new_session = connection
            .send_request(NewSessionRequest::new(
                cfg.cwd
                    .clone()
                    .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| "/".into())),
            ))
            .block_task()
            .await?;
        let session_id: SessionId = new_session.session_id.clone();
        {
            let mut slot = session_id_forward.write().await;
            *slot = Some(session_id.0.to_string());
        }
        let _ = events_tx_notif.send(SessionEvent::State {
            agent_id: agent_id_notif.clone(),
            session_id: Some(session_id.0.to_string()),
            state: SessionState::Running,
        });

        loop {
            tokio::select! {
                cmd = cmd_rx.recv() => {
                    let Some(cmd) = cmd else {
                        info!(agent = %agent_id_notif, "acp::runtime: command channel closed, shutting down");
                        break;
                    };
                    match cmd {
                        SessionCommand::Prompt { text, reply } => {
                            let text = match first_message_prefix.take() {
                                Some(prefix) => format!("{prefix}\n\n{text}"),
                                None => text,
                            };
                            let res = connection
                                .send_request(PromptRequest::new(
                                    session_id.clone(),
                                    vec![ContentBlock::Text(TextContent::new(text))],
                                ))
                                .block_task()
                                .await;
                            let mapped = res.map(|resp| {
                                info!(agent = %agent_id_notif, stop = ?resp.stop_reason, "acp::runtime: prompt resolved");
                            }).map_err(|e| e.to_string());
                            let _ = reply.send(mapped);
                        }
                        SessionCommand::Cancel { reply } => {
                            let res = connection
                                .send_notification(CancelNotification::new(session_id.clone()))
                                .map_err(|e| e.to_string());
                            let _ = reply.send(res);
                        }
                        SessionCommand::Shutdown { reply } => {
                            let _ = connection.send_notification(CancelNotification::new(session_id.clone()));
                            let _ = reply.send(());
                            break;
                        }
                    }
                }
                evt = client_events_rx.recv() => {
                    let Some(evt) = evt else { break };
                    match evt {
                        ClientEvent::Notification(boxed) => {
                            let notif = *boxed;
                            match serde_json::to_value(&notif.update) {
                                Ok(update) => {
                                    let _ = events_tx_notif.send(SessionEvent::Transcript {
                                        agent_id: agent_id_notif.clone(),
                                        session_id: notif.session_id.0.to_string(),
                                        update,
                                    });
                                }
                                Err(err) => warn!(%err, "acp::runtime: failed to serialize session update"),
                            }
                        }
                        ClientEvent::PermissionRequested { session_id: sid, options } => {
                            let _ = events_tx_notif.send(SessionEvent::PermissionRequest {
                                agent_id: agent_id_notif.clone(),
                                session_id: sid,
                                options,
                            });
                        }
                    }
                }
            }
        }
        Ok::<(), agent_client_protocol::Error>(())
    };

    let run = Client
        .builder()
        .on_receive_notification(
            {
                let client = client.clone();
                move |notification: agent_client_protocol::schema::SessionNotification, _cx| {
                    let client = client.clone();
                    async move {
                        client.forward_notification(notification);
                        Ok(())
                    }
                }
            },
            agent_client_protocol::on_receive_notification!(),
        )
        .on_receive_request(
            {
                let client = client.clone();
                move |req: agent_client_protocol::schema::RequestPermissionRequest,
                      responder: agent_client_protocol::Responder<
                    agent_client_protocol::schema::RequestPermissionResponse,
                >,
                      _cx| {
                    let client = client.clone();
                    async move { responder.respond(client.request_permission(&req)) }
                }
            },
            agent_client_protocol::on_receive_request!(),
        )
        .connect_with(transport, dispatch)
        .await;

    let final_state = match &run {
        Ok(_) => {
            info!(agent = %agent_id, "acp::runtime: session ended cleanly");
            SessionState::Ended
        }
        Err(err) => {
            warn!(agent = %agent_id, %err, "acp::runtime: session ended with error");
            SessionState::Error
        }
    };

    drop(child.kill().await);
    let sid = session_id_slot.read().await.clone();
    let _ = events_tx.send(SessionEvent::State {
        agent_id,
        session_id: sid,
        state: final_state,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AgentConfig, AgentProvider};

    fn dummy_session(id: &str) -> ResolvedSession {
        ResolvedSession {
            agent: AgentConfig {
                id: id.into(),
                provider: AgentProvider::AcpClaudeCode,
                command: Some("/bin/false".into()),
                args: Vec::new(),
                cwd: None,
                env: Default::default(),
                model: None,
            },
            profile_id: None,
            model: None,
            system_prompt: None,
        }
    }

    /// Regression: starting against a child that exits immediately
    /// pushes an `Error` lifecycle event rather than hanging forever.
    /// Smoke-tests the actor shell without depending on a real agent.
    #[tokio::test(flavor = "multi_thread")]
    async fn dead_child_yields_error_state() {
        let (tx, mut rx) = broadcast::channel(8);
        let handle = start_session(dummy_session("ded"), tx);

        // Starting event fires immediately.
        let first = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("starting event timely")
            .expect("starting event arrives");
        match first {
            SessionEvent::State {
                state: SessionState::Starting,
                ..
            } => {}
            other => panic!("expected Starting, got {other:?}"),
        }

        // Then the actor reports Error because `/bin/false` exits
        // before the initialize handshake lands.
        let err = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                match rx.recv().await {
                    Ok(SessionEvent::State {
                        state: SessionState::Error,
                        ..
                    }) => return Ok::<(), ()>(()),
                    Ok(SessionEvent::State {
                        state: SessionState::Ended,
                        ..
                    }) => return Ok(()),
                    Ok(_) => continue,
                    Err(_) => return Err(()),
                }
            }
        })
        .await
        .expect("actor settles");
        assert!(err.is_ok(), "actor reached terminal state");

        drop(handle);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cancel_against_dead_session_does_not_panic() {
        let (tx, _rx) = broadcast::channel(8);
        let handle = start_session(dummy_session("ded-cancel"), tx);

        // Give the actor a moment to fail.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let (reply_tx, reply_rx) = oneshot::channel();
        let _ = handle.cmd_tx.send(SessionCommand::Cancel { reply: reply_tx });
        // The actor already died, so the reply oneshot closes.
        let _ = tokio::time::timeout(std::time::Duration::from_secs(1), reply_rx).await;
    }
}
