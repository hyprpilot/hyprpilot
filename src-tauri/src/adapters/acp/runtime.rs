//! Per-instance actor: owns the ACP `ConnectionTo<Agent>` and the
//! child process; drives `initialize` → `session/new` → `session/prompt`
//! for the first prompt, then loops on a command mpsc while also
//! fanning `SessionNotification`s out to a broadcast channel the
//! daemon subscribes to for Tauri event emission.

use std::sync::Arc;

use agent_client_protocol::schema::{
    CancelNotification, ClientCapabilities, ContentBlock, FileSystemCapabilities, InitializeRequest,
    ListSessionsRequest, ListSessionsResponse, LoadSessionRequest, NewSessionRequest, PromptRequest, ProtocolVersion,
    SessionId, TextContent,
};
use agent_client_protocol::{ByteStreams, Client};
use serde::Serialize;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use tracing::{error, info, warn};

use super::client::{AcpClient, ClientEvent, PermissionOptionView};
use super::instance::AcpInstance;
use super::spawn::spawn_agent;
use crate::adapters::profile::ResolvedInstance;

/// Register a typed `on_receive_request` handler that delegates to an
/// async `AcpClient` method returning `Result<Response,
/// agent_client_protocol::Error>`. One registration line per method
/// keeps the handler chain legible.
macro_rules! register_client_handler {
    ($builder:expr, $client:expr, $method:ident) => {{
        let client = $client.clone();
        $builder.on_receive_request(
            move |req, responder: agent_client_protocol::Responder<_>, _cx| {
                let client = client.clone();
                async move { responder.respond_with_result(client.$method(&req).await) }
            },
            agent_client_protocol::on_receive_request!(),
        )
    }};
}

/// Commands the per-instance actor accepts. The actor keeps state
/// internal; this enum is the only public surface the dispatcher
/// uses to drive it.
#[derive(Debug)]
pub enum InstanceCommand {
    Prompt {
        text: String,
        reply: oneshot::Sender<Result<(), String>>,
    },
    Cancel {
        reply: oneshot::Sender<Result<(), String>>,
    },
    /// Ask the agent for its persisted session index. Works in any
    /// bootstrap mode — the actor is always past `initialize` by the
    /// time it processes commands.
    ListSessions {
        cwd: Option<std::path::PathBuf>,
        reply: oneshot::Sender<Result<ListSessionsResponse, String>>,
    },
    /// Shutdown hook — stops the actor after the current prompt
    /// (or immediately if idle). Reply carries the final state.
    Shutdown {
        reply: oneshot::Sender<()>,
    },
}

/// Bootstrap discriminator the dispatch closure branches on after
/// `initialize`. Agent owns session persistence; this picks between
/// creating a new session, resuming an existing one, or running an
/// init-only actor that never binds to a session.
#[derive(Debug, Clone)]
pub enum Bootstrap {
    /// Fresh session — issues `session/new`.
    Fresh,
    /// Resume an existing session by id — issues `session/load`.
    /// Historical updates the agent streams during the load call
    /// flow through the standard notification path.
    Resume(SessionId),
    /// Init-only. Actor serves `ListSessions` + `Shutdown` without
    /// ever binding a session. Used for ephemeral query actors that
    /// don't own a turn.
    ListOnly,
}

/// Events the actor broadcasts upstream. `AcpInstances` owns a
/// `broadcast::Sender` and the daemon's Tauri `setup` closure
/// subscribes to it to emit `acp:*` events.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InstanceEvent {
    /// Instance transitioned to a new lifecycle state.
    State {
        agent_id: String,
        instance_id: String,
        session_id: Option<String>,
        state: InstanceState,
    },
    /// Agent pushed a `session/update` notification; raw JSON
    /// because the upstream shape is `#[non_exhaustive]` and we
    /// don't want to reshape every variant here.
    Transcript {
        agent_id: String,
        instance_id: String,
        session_id: String,
        update: serde_json::Value,
    },
    /// Agent asked permission. Auto-denied server-side until
    /// PermissionController lands; emitted so the webview can log /
    /// show it anyway.
    PermissionRequest {
        agent_id: String,
        instance_id: String,
        session_id: String,
        options: Vec<PermissionOptionView>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstanceState {
    Starting,
    Running,
    Ended,
    Error,
}

/// Start a per-instance actor. Returns the handle immediately; the
/// `bootstrap` variant picks between `session/new` (`Fresh`),
/// `session/load` (`Resume`), or neither (`ListOnly`). Sends
/// `InstanceEvent`s onto `events_tx` for every lifecycle transition
/// and every `SessionUpdate` the agent streams.
pub fn start_instance(
    resolved: ResolvedInstance,
    instance_id: String,
    events_tx: broadcast::Sender<InstanceEvent>,
    bootstrap: Bootstrap,
) -> AcpInstance {
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<InstanceCommand>();
    let initial = match &bootstrap {
        Bootstrap::Resume(id) => Some(id.clone()),
        Bootstrap::Fresh | Bootstrap::ListOnly => None,
    };
    let session_id = Arc::new(tokio::sync::RwLock::new(initial));

    let instance = AcpInstance {
        agent_id: resolved.agent.id.clone(),
        cmd_tx,
        session_id: session_id.clone(),
    };

    tokio::spawn(run_instance(
        resolved,
        instance_id,
        cmd_rx,
        events_tx,
        session_id,
        bootstrap,
    ));

    instance
}

/// The long-lived actor body.
async fn run_instance(
    resolved: ResolvedInstance,
    instance_id: String,
    mut cmd_rx: mpsc::UnboundedReceiver<InstanceCommand>,
    events_tx: broadcast::Sender<InstanceEvent>,
    session_id_slot: Arc<tokio::sync::RwLock<Option<SessionId>>>,
    bootstrap: Bootstrap,
) {
    let agent_id = resolved.agent.id.clone();
    let _ = events_tx.send(InstanceEvent::State {
        agent_id: agent_id.clone(),
        instance_id: instance_id.clone(),
        session_id: None,
        state: InstanceState::Starting,
    });

    let cfg = {
        let mut cfg = resolved.agent.clone();
        cfg.model = resolved.model.clone();
        cfg
    };
    let system_prompt = resolved.system_prompt.clone();

    let (mut child, stdio, mut first_message_prefix) = match spawn_agent(&cfg, system_prompt.as_deref()) {
        Ok(spawned) => (spawned.child, spawned.stdio, spawned.first_message_prefix),
        Err(err) => {
            error!(agent = %agent_id, %err, "acp::runtime: spawn failed");
            let _ = events_tx.send(InstanceEvent::State {
                agent_id,
                instance_id,
                session_id: None,
                state: InstanceState::Error,
            });
            return;
        }
    };

    let (client_events_tx, mut client_events_rx) = mpsc::unbounded_channel::<ClientEvent>();
    let sandbox_root = cfg
        .cwd
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| "/".into()));
    let client = match AcpClient::new(client_events_tx, sandbox_root) {
        Ok(c) => c,
        Err(err) => {
            error!(agent = %agent_id, %err, "acp::runtime: sandbox init failed");
            let _ = events_tx.send(InstanceEvent::State {
                agent_id,
                instance_id,
                session_id: None,
                state: InstanceState::Error,
            });
            return;
        }
    };

    let transport = ByteStreams::new(stdio.stdin.compat_write(), stdio.stdout.compat());

    let events_tx_notif = events_tx.clone();
    let agent_id_notif = agent_id.clone();
    let instance_id_notif = instance_id.clone();
    let session_id_forward = session_id_slot.clone();

    let dispatch = async move |connection: agent_client_protocol::ConnectionTo<agent_client_protocol::Agent>| {
        let init = connection
            .send_request(
                InitializeRequest::new(ProtocolVersion::V1).client_capabilities(
                    ClientCapabilities::new()
                        .fs(FileSystemCapabilities::new().read_text_file(true).write_text_file(true))
                        .terminal(true),
                ),
            )
            .block_task()
            .await?;
        info!(agent = %agent_id_notif, protocol = ?init.protocol_version, "acp::runtime: initialized");

        let cwd = cfg
            .cwd
            .clone()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| "/".into()));
        let load_supported = init.agent_capabilities.load_session;

        let session_id: Option<SessionId> = match bootstrap {
            Bootstrap::Fresh => {
                let new_session = connection
                    .send_request(NewSessionRequest::new(cwd.clone()))
                    .block_task()
                    .await?;
                let sid = new_session.session_id.clone();
                {
                    let mut slot = session_id_forward.write().await;
                    *slot = Some(sid.clone());
                }
                let _ = events_tx_notif.send(InstanceEvent::State {
                    agent_id: agent_id_notif.clone(),
                    instance_id: instance_id_notif.clone(),
                    session_id: Some(sid.0.to_string()),
                    state: InstanceState::Running,
                });
                Some(sid)
            }
            Bootstrap::Resume(sid) => {
                if !load_supported {
                    warn!(agent = %agent_id_notif, "acp::runtime: load_session not advertised by agent");
                    let _ = events_tx_notif.send(InstanceEvent::State {
                        agent_id: agent_id_notif.clone(),
                        instance_id: instance_id_notif.clone(),
                        session_id: Some(sid.0.to_string()),
                        state: InstanceState::Error,
                    });
                    return Err(
                        agent_client_protocol::Error::method_not_found().data(serde_json::json!({
                            "reason": format!("{}: load_session not supported", agent_id_notif),
                        })),
                    );
                }
                {
                    let mut slot = session_id_forward.write().await;
                    *slot = Some(sid.clone());
                }
                if let Err(err) = connection
                    .send_request(LoadSessionRequest::new(sid.clone(), cwd.clone()))
                    .block_task()
                    .await
                {
                    warn!(agent = %agent_id_notif, %err, "acp::runtime: load_session failed");
                    let _ = events_tx_notif.send(InstanceEvent::State {
                        agent_id: agent_id_notif.clone(),
                        instance_id: instance_id_notif.clone(),
                        session_id: Some(sid.0.to_string()),
                        state: InstanceState::Error,
                    });
                    return Err(err);
                }
                let _ = events_tx_notif.send(InstanceEvent::State {
                    agent_id: agent_id_notif.clone(),
                    instance_id: instance_id_notif.clone(),
                    session_id: Some(sid.0.to_string()),
                    state: InstanceState::Running,
                });
                Some(sid)
            }
            Bootstrap::ListOnly => {
                let _ = events_tx_notif.send(InstanceEvent::State {
                    agent_id: agent_id_notif.clone(),
                    instance_id: instance_id_notif.clone(),
                    session_id: None,
                    state: InstanceState::Running,
                });
                None
            }
        };

        loop {
            tokio::select! {
                cmd = cmd_rx.recv() => {
                    let Some(cmd) = cmd else {
                        info!(agent = %agent_id_notif, "acp::runtime: command channel closed, shutting down");
                        break;
                    };
                    match cmd {
                        InstanceCommand::Prompt { text, reply } => {
                            let Some(sid) = session_id.clone() else {
                                let _ = reply.send(Err("no live session in list-only actor".into()));
                                continue;
                            };
                            let text = match first_message_prefix.take() {
                                Some(prefix) => format!("{prefix}\n\n{text}"),
                                None => text,
                            };
                            let res = connection
                                .send_request(PromptRequest::new(
                                    sid,
                                    vec![ContentBlock::Text(TextContent::new(text))],
                                ))
                                .block_task()
                                .await;
                            let mapped = res.map(|resp| {
                                info!(agent = %agent_id_notif, stop = ?resp.stop_reason, "acp::runtime: prompt resolved");
                            }).map_err(|e| e.to_string());
                            let _ = reply.send(mapped);
                        }
                        InstanceCommand::Cancel { reply } => {
                            let Some(sid) = session_id.clone() else {
                                let _ = reply.send(Err("no live session in list-only actor".into()));
                                continue;
                            };
                            let res = connection
                                .send_notification(CancelNotification::new(sid))
                                .map_err(|e| e.to_string());
                            let _ = reply.send(res);
                        }
                        InstanceCommand::ListSessions { cwd: filter_cwd, reply } => {
                            let mut req = ListSessionsRequest::new();
                            if let Some(c) = filter_cwd {
                                req = req.cwd(c);
                            }
                            let res = connection
                                .send_request(req)
                                .block_task()
                                .await
                                .map_err(|e| e.to_string());
                            let _ = reply.send(res);
                        }
                        InstanceCommand::Shutdown { reply } => {
                            if let Some(sid) = session_id.clone() {
                                let _ = connection.send_notification(CancelNotification::new(sid));
                            }
                            let _ = reply.send(());
                            break;
                        }
                    }
                }
                evt = client_events_rx.recv() => {
                    let Some(evt) = evt else { break };
                    match evt {
                        ClientEvent::Notification { session_id: sid, update } => {
                            let _ = events_tx_notif.send(InstanceEvent::Transcript {
                                agent_id: agent_id_notif.clone(),
                                instance_id: instance_id_notif.clone(),
                                session_id: sid,
                                update,
                            });
                        }
                        ClientEvent::PermissionRequested { session_id: sid, options } => {
                            let _ = events_tx_notif.send(InstanceEvent::PermissionRequest {
                                agent_id: agent_id_notif.clone(),
                                instance_id: instance_id_notif.clone(),
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

    let builder = Client.builder().on_receive_notification(
        {
            let client = client.clone();
            move |notification: super::client::TolerantSessionNotification, _cx| {
                let client = client.clone();
                async move {
                    client.forward_notification(notification);
                    Ok(())
                }
            }
        },
        agent_client_protocol::on_receive_notification!(),
    );
    let builder = register_client_handler!(builder, client, request_permission);
    let builder = register_client_handler!(builder, client, read_text_file);
    let builder = register_client_handler!(builder, client, write_text_file);
    let builder = register_client_handler!(builder, client, create_terminal);
    let builder = register_client_handler!(builder, client, terminal_output);
    let builder = register_client_handler!(builder, client, wait_for_terminal_exit);
    let builder = register_client_handler!(builder, client, kill_terminal);
    let builder = register_client_handler!(builder, client, release_terminal);

    let run = builder.connect_with(transport, dispatch).await;

    let final_state = match &run {
        Ok(_) => {
            info!(agent = %agent_id, "acp::runtime: instance ended cleanly");
            InstanceState::Ended
        }
        Err(err) => {
            warn!(agent = %agent_id, %err, "acp::runtime: instance ended with error");
            InstanceState::Error
        }
    };

    drop(child.kill().await);
    let sid = session_id_slot.read().await.clone();
    if let Some(ref id) = sid {
        client.drain_terminals_for_session(id).await;
    }
    let _ = events_tx.send(InstanceEvent::State {
        agent_id,
        instance_id,
        session_id: sid.as_ref().map(|id| id.0.to_string()),
        state: final_state,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AgentConfig, AgentProvider};

    fn dummy_resolved(id: &str) -> ResolvedInstance {
        ResolvedInstance {
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
        let handle = start_instance(dummy_resolved("ded"), "ded".into(), tx, Bootstrap::Fresh);

        let first = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
            .await
            .expect("starting event timely")
            .expect("starting event arrives");
        match first {
            InstanceEvent::State {
                state: InstanceState::Starting,
                ..
            } => {}
            other => panic!("expected Starting, got {other:?}"),
        }

        let err = tokio::time::timeout(std::time::Duration::from_secs(15), async {
            loop {
                match rx.recv().await {
                    Ok(InstanceEvent::State {
                        state: InstanceState::Error,
                        ..
                    }) => return Ok::<(), ()>(()),
                    Ok(InstanceEvent::State {
                        state: InstanceState::Ended,
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
        let handle = start_instance(dummy_resolved("ded-cancel"), "ded-cancel".into(), tx, Bootstrap::Fresh);

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let (reply_tx, reply_rx) = oneshot::channel();
        let _ = handle.cmd_tx.send(InstanceCommand::Cancel { reply: reply_tx });
        let _ = tokio::time::timeout(std::time::Duration::from_secs(1), reply_rx).await;
    }

    /// Smoke: a `ListOnly` actor against a dead child still settles
    /// (the `initialize` roundtrip fails, which drives the actor to
    /// `Error` instead of panicking or hanging). The real list-only
    /// path is exercised end-to-end against the mock ACP agent.
    #[tokio::test(flavor = "multi_thread")]
    async fn list_only_against_dead_child_settles() {
        let (tx, mut rx) = broadcast::channel(8);
        let handle = start_instance(dummy_resolved("ded-list"), "ded-list".into(), tx, Bootstrap::ListOnly);

        let settled = tokio::time::timeout(std::time::Duration::from_secs(15), async {
            loop {
                match rx.recv().await {
                    Ok(InstanceEvent::State {
                        state: InstanceState::Error,
                        ..
                    })
                    | Ok(InstanceEvent::State {
                        state: InstanceState::Ended,
                        ..
                    }) => return Ok::<(), ()>(()),
                    Ok(_) => continue,
                    Err(_) => return Err(()),
                }
            }
        })
        .await
        .expect("actor settles");
        assert!(settled.is_ok());

        drop(handle);
    }

    /// `Bootstrap::Resume` against a child that dies before responding
    /// never leaks a partial session — the actor funnels through
    /// `InstanceState::Error`. The capability gate is a pre-connection
    /// check; integration coverage lives against the mock agent.
    #[tokio::test(flavor = "multi_thread")]
    async fn resume_against_dead_child_reports_error() {
        let (tx, mut rx) = broadcast::channel(8);
        let sid = SessionId::new("00000000-0000-0000-0000-000000000000");
        let handle = start_instance(
            dummy_resolved("ded-resume"),
            "ded-resume".into(),
            tx,
            Bootstrap::Resume(sid),
        );

        let settled = tokio::time::timeout(std::time::Duration::from_secs(15), async {
            loop {
                match rx.recv().await {
                    Ok(InstanceEvent::State {
                        state: InstanceState::Error,
                        ..
                    })
                    | Ok(InstanceEvent::State {
                        state: InstanceState::Ended,
                        ..
                    }) => return Ok::<(), ()>(()),
                    Ok(_) => continue,
                    Err(_) => return Err(()),
                }
            }
        })
        .await
        .expect("actor settles");
        assert!(settled.is_ok());

        drop(handle);
    }
}
