//! Per-instance actor: owns the ACP `ConnectionTo<Agent>` and the
//! child process; drives `initialize` → `session/new` → `session/prompt`
//! for the first prompt, then loops on a command mpsc while also
//! fanning `SessionNotification`s out to a broadcast channel the
//! daemon subscribes to for Tauri event emission.

use std::sync::Arc;

use agent_client_protocol::schema::{
    CancelNotification, ClientCapabilities, FileSystemCapabilities, InitializeRequest, ListSessionsRequest,
    ListSessionsResponse, LoadSessionRequest, NewSessionRequest, PromptRequest, ProtocolVersion, SessionId,
};
use agent_client_protocol::{ByteStreams, Client};
use serde::Serialize;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};
use tracing::{debug, error, info, warn};

use super::client::{AcpClient, ClientEvent};
use super::instance::AcpInstance;
use super::spawn::spawn_agent;
use crate::adapters::permission::{PermissionController, PermissionOptionView};
use crate::adapters::profile::ResolvedInstance;
use crate::config::ProfileConfig;

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
        attachments: Vec<crate::adapters::Attachment>,
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
#[serde(tag = "event", rename_all = "snake_case")]
pub enum InstanceEvent {
    /// Instance transitioned to a new lifecycle state.
    State {
        agent_id: String,
        instance_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
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
        /// Active turn id while the actor is processing a `Prompt`
        /// command; `None` for spontaneous notifications outside
        /// any in-flight turn.
        turn_id: Option<String>,
        update: serde_json::Value,
    },
    /// Agent asked permission and the controller bounced to the UI.
    /// Profile auto-accept / auto-reject decisions do NOT emit this —
    /// they resolve inside `AcpClient::request_permission` without
    /// surfacing to the webview.
    PermissionRequest {
        agent_id: String,
        instance_id: String,
        session_id: String,
        turn_id: Option<String>,
        request_id: String,
        tool: String,
        kind: String,
        args: String,
        options: Vec<PermissionOptionView>,
    },
    /// Actor accepted a new `Prompt` command and is about to send
    /// `session/prompt`. `turn_id` is a fresh UUID stamped onto
    /// every subsequent `Transcript` / `PermissionRequest` the actor
    /// emits until the matching `TurnEnded` lands.
    TurnStarted {
        agent_id: String,
        instance_id: String,
        session_id: String,
        turn_id: String,
    },
    /// `session/prompt` resolved (or errored). `stop_reason` mirrors
    /// the ACP `StopReason` wire string on success; `None` on error
    /// or cancellation.
    TurnEnded {
        agent_id: String,
        instance_id: String,
        session_id: String,
        turn_id: String,
        stop_reason: Option<String>,
    },
    /// Registry membership changed — spawn / shutdown / restart.
    /// Emitted by `AcpInstances`, not the per-instance actor.
    InstancesChanged {
        instance_ids: Vec<String>,
        focused_id: Option<String>,
    },
    /// Focus pointer moved (explicit `focus` call or auto-focus on
    /// shutdown of the focused instance).
    InstancesFocused { instance_id: Option<String> },
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
    key: crate::adapters::InstanceKey,
    profile_id: Option<String>,
    events_tx: broadcast::Sender<InstanceEvent>,
    bootstrap: Bootstrap,
    permissions: Arc<dyn PermissionController>,
    profile: Option<ProfileConfig>,
) -> AcpInstance {
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel::<InstanceCommand>();
    let initial = match &bootstrap {
        Bootstrap::Resume(id) => Some(id.clone()),
        Bootstrap::Fresh | Bootstrap::ListOnly => None,
    };
    let session_id = Arc::new(tokio::sync::RwLock::new(initial));
    let mode = resolved.mode.clone();
    let instance_id = key.as_string();

    // Mode is a per-instance operational override (e.g. claude-code's
    // `plan` / `edit`). Adapter carries it through to the runtime
    // tracing span + InstanceInfo so UI pickers see it. Vendor-specific
    // wire injection (ACP `_meta` field, CLI flag, etc.) lands in the
    // vendor-agent impl; today we surface it here.
    if let Some(m) = &mode {
        tracing::info!(
            agent = %resolved.agent.id,
            instance = %instance_id,
            mode = %m,
            "acp::runtime: instance mode set"
        );
    }

    let instance = AcpInstance {
        key,
        agent_id: resolved.agent.id.clone(),
        profile_id,
        mode,
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
        permissions,
        profile,
    ));

    instance
}

/// The long-lived actor body.
#[allow(clippy::too_many_arguments)]
async fn run_instance(
    resolved: ResolvedInstance,
    instance_id: String,
    mut cmd_rx: mpsc::UnboundedReceiver<InstanceCommand>,
    events_tx: broadcast::Sender<InstanceEvent>,
    session_id_slot: Arc<tokio::sync::RwLock<Option<SessionId>>>,
    bootstrap: Bootstrap,
    permissions: Arc<dyn PermissionController>,
    profile: Option<ProfileConfig>,
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

    let (mut child, stdio, stderr, mut first_message_prefix) = match spawn_agent(&cfg, system_prompt.as_deref()) {
        Ok(spawned) => (
            spawned.child,
            spawned.stdio,
            spawned.stderr,
            spawned.first_message_prefix,
        ),
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

    // Drain the subprocess's stderr into tracing so vendor-SDK cleanup
    // noise lands in our rolling log file instead of the parent
    // terminal. Each line goes through at `info!` with an `agent_stderr`
    // target so users can filter via
    // `RUST_LOG=hyprpilot=info,agent_stderr=warn`. The task ends when
    // the stream closes (child exit).
    {
        use tokio::io::{AsyncBufReadExt, BufReader};
        let agent_for_stderr = agent_id.clone();
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            loop {
                match lines.next_line().await {
                    Ok(Some(line)) => {
                        tracing::info!(target: "agent_stderr", agent = %agent_for_stderr, "{line}");
                    }
                    Ok(None) => break,
                    Err(err) => {
                        tracing::warn!(
                            target: "agent_stderr",
                            agent = %agent_for_stderr,
                            %err,
                            "stderr read error"
                        );
                        break;
                    }
                }
            }
        });
    }

    // Tee stdout → tracing + ACP transport. Stdout IS the ACP wire
    // channel so we can't just redirect it; we read each line, emit it
    // at `trace!` with target `agent_stdout`, then forward the original
    // bytes into a duplex pipe the transport reads from. Filter in with
    // `RUST_LOG=agent_stdout=trace`; noisy (every JSON-RPC frame), so
    // `trace` is deliberately opt-in.
    let transport_stdout = {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        let (mut tee_writer, tee_reader) = tokio::io::duplex(64 * 1024);
        let agent_for_stdout = agent_id.clone();
        let child_stdout = stdio.stdout;
        tokio::spawn(async move {
            let mut reader = BufReader::new(child_stdout);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => break,
                    Ok(_) => {
                        let trimmed = line.trim_end_matches(['\n', '\r']);
                        if !trimmed.is_empty() {
                            tracing::trace!(target: "agent_stdout", agent = %agent_for_stdout, "{trimmed}");
                        }
                        if let Err(err) = tee_writer.write_all(line.as_bytes()).await {
                            tracing::warn!(
                                target: "agent_stdout",
                                agent = %agent_for_stdout,
                                %err,
                                "tee forward failed"
                            );
                            break;
                        }
                    }
                    Err(err) => {
                        tracing::warn!(
                            target: "agent_stdout",
                            agent = %agent_for_stdout,
                            %err,
                            "stdout read error"
                        );
                        break;
                    }
                }
            }
        });
        tee_reader
    };

    let (client_events_tx, mut client_events_rx) = mpsc::unbounded_channel::<ClientEvent>();
    let sandbox_root = cfg
        .cwd
        .clone()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| "/".into()));
    let client = match AcpClient::with_instance_id(
        client_events_tx,
        sandbox_root,
        permissions.clone(),
        profile.clone(),
        Some(instance_id.clone()),
    ) {
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

    let transport = ByteStreams::new(stdio.stdin.compat_write(), transport_stdout.compat());

    let events_tx_notif = events_tx.clone();
    let agent_id_notif = agent_id.clone();
    let instance_id_notif = instance_id.clone();
    let session_id_forward = session_id_slot.clone();
    // Tracks the in-flight turn id so the notification / permission
    // arms of the dispatch loop can stamp events with it without
    // re-coordinating with the Prompt-handling task. Set when a
    // `Prompt` is accepted, cleared when the spawned `session/prompt`
    // task replies. `tokio::sync::RwLock` because the spawned task
    // crosses an `.await`; reads from the loop are non-blocking.
    let current_turn_id: Arc<tokio::sync::RwLock<Option<String>>> = Arc::new(tokio::sync::RwLock::new(None));

    let dispatch = async move |connection: agent_client_protocol::ConnectionTo<agent_client_protocol::Agent>| {
        debug!(agent = %agent_id_notif, "acp::runtime: sending initialize request");
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
        info!(
            agent = %agent_id_notif,
            protocol = ?init.protocol_version,
            load_session = init.agent_capabilities.load_session,
            "acp::runtime: initialized"
        );

        let cwd = cfg
            .cwd
            .clone()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| "/".into()));
        let load_supported = init.agent_capabilities.load_session;

        let session_id: Option<SessionId> = match bootstrap {
            Bootstrap::Fresh => {
                debug!(agent = %agent_id_notif, "acp::runtime: sending session/new");
                let new_session = connection
                    .send_request(NewSessionRequest::new(cwd.clone()))
                    .block_task()
                    .await?;
                let sid = new_session.session_id.clone();
                info!(
                    agent = %agent_id_notif,
                    instance = %instance_id_notif,
                    session = %sid,
                    "acp::runtime: session/new accepted"
                );
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
                debug!(agent = %agent_id_notif, session = %sid, "acp::runtime: sending session/load");
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
                info!(
                    agent = %agent_id_notif,
                    instance = %instance_id_notif,
                    session = %sid,
                    "acp::runtime: session/load accepted"
                );
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
                        // Detached: awaiting `send_request(...).block_task()` inline here
                        // blocks the select! from pumping `client_events_rx`, so every
                        // `SessionNotification` (and every `PermissionRequest`!) queues
                        // on the mpsc until the prompt resolves. The permission path
                        // blocks for up to 10min waiting on a UI reply — but the UI
                        // never sees the prompt because the event is stuck in that same
                        // mpsc. Spawn the request so the loop keeps draining.
                        InstanceCommand::Prompt { text, attachments, reply } => {
                            let Some(sid) = session_id.clone() else {
                                let _ = reply.send(Err("no live session in list-only actor".into()));
                                continue;
                            };
                            let text = match first_message_prefix.take() {
                                Some(prefix) => format!("{prefix}\n\n{text}"),
                                None => text,
                            };
                            let turn_id = uuid::Uuid::new_v4().to_string();
                            info!(
                                agent = %agent_id_notif,
                                session = %sid,
                                turn = %turn_id,
                                text_len = text.len(),
                                attachments = attachments.len(),
                                "acp::runtime: turn start (session/prompt)"
                            );
                            *current_turn_id.write().await = Some(turn_id.clone());
                            let _ = events_tx_notif.send(InstanceEvent::TurnStarted {
                                agent_id: agent_id_notif.clone(),
                                instance_id: instance_id_notif.clone(),
                                session_id: sid.0.to_string(),
                                turn_id: turn_id.clone(),
                            });
                            let blocks = super::mapping::build_prompt_blocks(&text, &attachments);
                            let conn = connection.clone();
                            let agent_log = agent_id_notif.clone();
                            let session_log = sid.clone();
                            let events_tx_done = events_tx_notif.clone();
                            let current_turn_done = current_turn_id.clone();
                            let agent_id_done = agent_id_notif.clone();
                            let instance_id_done = instance_id_notif.clone();
                            let turn_id_done = turn_id.clone();
                            tokio::spawn(async move {
                                let res = conn
                                    .send_request(PromptRequest::new(sid.clone(), blocks))
                                    .block_task()
                                    .await;
                                let (stop_reason, mapped) = match res {
                                    Ok(resp) => {
                                        info!(
                                            agent = %agent_log,
                                            session = %session_log,
                                            turn = %turn_id_done,
                                            stop_reason = ?resp.stop_reason,
                                            "acp::runtime: turn stop (prompt resolved)"
                                        );
                                        let stop = serde_json::to_value(resp.stop_reason)
                                            .ok()
                                            .and_then(|v| v.as_str().map(str::to_owned));
                                        (stop, Ok(()))
                                    }
                                    Err(err) => {
                                        warn!(
                                            agent = %agent_log,
                                            session = %session_log,
                                            turn = %turn_id_done,
                                            %err,
                                            "acp::runtime: turn ended with error"
                                        );
                                        (None, Err(err.to_string()))
                                    }
                                };
                                {
                                    let mut slot = current_turn_done.write().await;
                                    if slot.as_deref() == Some(turn_id_done.as_str()) {
                                        *slot = None;
                                    }
                                }
                                let _ = events_tx_done.send(InstanceEvent::TurnEnded {
                                    agent_id: agent_id_done,
                                    instance_id: instance_id_done,
                                    session_id: sid.0.to_string(),
                                    turn_id: turn_id_done,
                                    stop_reason,
                                });
                                let _ = reply.send(mapped);
                            });
                        }
                        InstanceCommand::Cancel { reply } => {
                            let Some(sid) = session_id.clone() else {
                                let _ = reply.send(Err("no live session in list-only actor".into()));
                                continue;
                            };
                            info!(
                                agent = %agent_id_notif,
                                session = %sid,
                                "acp::runtime: turn cancel (CancelNotification)"
                            );
                            let res = connection
                                .send_notification(CancelNotification::new(sid))
                                .map_err(|e| e.to_string());
                            let _ = reply.send(res);
                        }
                        // Detached for the same reason as Prompt: list_sessions can take
                        // seconds against a remote index, and blocking the select! starves
                        // event pumping.
                        InstanceCommand::ListSessions { cwd: filter_cwd, reply } => {
                            debug!(
                                agent = %agent_id_notif,
                                cwd_filter = ?filter_cwd,
                                "acp::runtime: session/list requested"
                            );
                            let conn = connection.clone();
                            tokio::spawn(async move {
                                let mut req = ListSessionsRequest::new();
                                if let Some(c) = filter_cwd {
                                    req = req.cwd(c);
                                }
                                let res = conn
                                    .send_request(req)
                                    .block_task()
                                    .await
                                    .map_err(|e| e.to_string());
                                let _ = reply.send(res);
                            });
                        }
                        InstanceCommand::Shutdown { reply } => {
                            info!(
                                agent = %agent_id_notif,
                                instance = %instance_id_notif,
                                has_session = session_id.is_some(),
                                reason = "shutdown command received",
                                "acp::runtime: shutting down instance"
                            );
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
                            let update_kind = update
                                .get("sessionUpdate")
                                .and_then(|v| v.as_str())
                                .unwrap_or("<unknown>");
                            if update_kind == "agent_message_chunk" || update_kind == "user_message_chunk" {
                                let chunk_len = update
                                    .get("content")
                                    .and_then(|c| c.get("text"))
                                    .and_then(|v| v.as_str())
                                    .map(str::len)
                                    .unwrap_or(0);
                                tracing::trace!(
                                    agent = %agent_id_notif,
                                    session = %sid,
                                    update_kind,
                                    chunk_len,
                                    "acp::runtime: session/update text chunk"
                                );
                            } else {
                                debug!(
                                    agent = %agent_id_notif,
                                    session = %sid,
                                    update_kind,
                                    "acp::runtime: session/update received"
                                );
                            }
                            let turn_id = current_turn_id.read().await.clone();
                            let _ = events_tx_notif.send(InstanceEvent::Transcript {
                                agent_id: agent_id_notif.clone(),
                                instance_id: instance_id_notif.clone(),
                                session_id: sid,
                                turn_id,
                                update,
                            });
                        }
                        ClientEvent::PermissionRequested {
                            session_id: sid,
                            request_id,
                            tool,
                            kind,
                            args,
                            options,
                        } => {
                            debug!(
                                agent = %agent_id_notif,
                                session = %sid,
                                request_id,
                                tool = %tool,
                                "acp::runtime: fan out permission prompt to UI"
                            );
                            let turn_id = current_turn_id.read().await.clone();
                            let _ = events_tx_notif.send(InstanceEvent::PermissionRequest {
                                agent_id: agent_id_notif.clone(),
                                instance_id: instance_id_notif.clone(),
                                session_id: sid,
                                turn_id,
                                request_id,
                                tool,
                                kind,
                                args,
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

    // Give the agent subprocess a brief window to exit cleanly after
    // the transport closes above. The `CancelNotification` we sent on
    // shutdown + the resulting stdin EOF are the standard ACP signals
    // to terminate. SIGKILL'ing zero-delay mid-cleanup makes vendor
    // SDKs (notably `@anthropic-ai/claude-agent-sdk` inside
    // claude-code-acp) spew "Query closed before response received" on
    // stderr because they're tearing down a still-open Anthropic
    // streaming connection that's kept warm between turns. Wait up to
    // 5s for a clean exit, fall back to SIGKILL.
    match tokio::time::timeout(std::time::Duration::from_secs(5), child.wait()).await {
        Ok(Ok(status)) => debug!(agent = %agent_id, ?status, "acp::runtime: child exited cleanly"),
        Ok(Err(err)) => warn!(agent = %agent_id, %err, "acp::runtime: child wait failed"),
        Err(_) => {
            warn!(
                agent = %agent_id,
                "acp::runtime: child did not exit within 5s after stdin EOF, sending SIGKILL"
            );
            let _ = child.kill().await;
        }
    }
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
    use crate::adapters::permission::DefaultPermissionController;
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
            mode: None,
        }
    }

    fn dummy_permissions() -> Arc<dyn PermissionController> {
        Arc::new(DefaultPermissionController::new()) as Arc<dyn PermissionController>
    }

    /// Regression: starting against a child that exits immediately
    /// pushes an `Error` lifecycle event rather than hanging forever.
    /// Smoke-tests the actor shell without depending on a real agent.
    #[tokio::test(flavor = "multi_thread")]
    async fn dead_child_yields_error_state() {
        let (tx, mut rx) = broadcast::channel(8);
        let handle = start_instance(
            dummy_resolved("ded"),
            crate::adapters::InstanceKey::new_v4(),
            None,
            tx,
            Bootstrap::Fresh,
            dummy_permissions(),
            None,
        );

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
        let handle = start_instance(
            dummy_resolved("ded-cancel"),
            crate::adapters::InstanceKey::new_v4(),
            None,
            tx,
            Bootstrap::Fresh,
            dummy_permissions(),
            None,
        );

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
        let handle = start_instance(
            dummy_resolved("ded-list"),
            crate::adapters::InstanceKey::new_v4(),
            None,
            tx,
            Bootstrap::ListOnly,
            dummy_permissions(),
            None,
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

    /// Regression for the "LLM responses don't show" bug: awaiting a
    /// long-running request inline inside the select! arm blocks the
    /// event-forwarding arm on the same loop, starving transcript +
    /// permission-request fanout. The fix detaches the request into
    /// its own `tokio::spawn` so the loop keeps polling
    /// `client_events_rx`. This test models the select!'s contract on
    /// pure channels (no real ACP connection needed) — an inline
    /// `.await` against a 10s-blocking "request" must not delay an
    /// event pushed on the sibling channel mid-flight.
    #[tokio::test(start_paused = true)]
    async fn select_loop_pumps_events_while_request_outstanding() {
        use tokio::sync::{mpsc, oneshot};

        enum Cmd {
            Request { reply: oneshot::Sender<()> },
        }

        let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<Cmd>();
        let (evt_tx, mut evt_rx) = mpsc::unbounded_channel::<&'static str>();
        let (observed_tx, mut observed_rx) = mpsc::unbounded_channel::<&'static str>();

        let loop_handle = tokio::spawn(async move {
            loop {
                tokio::select! {
                    cmd = cmd_rx.recv() => {
                        let Some(cmd) = cmd else { break };
                        match cmd {
                            // Same shape as the fixed `Prompt` arm: spawn, do not await.
                            Cmd::Request { reply } => {
                                tokio::spawn(async move {
                                    tokio::time::sleep(std::time::Duration::from_secs(10)).await;
                                    let _ = reply.send(());
                                });
                            }
                        }
                    }
                    evt = evt_rx.recv() => {
                        let Some(evt) = evt else { break };
                        let _ = observed_tx.send(evt);
                    }
                }
            }
        });

        // 1. Submit a long-running "request".
        let (reply_tx, reply_rx) = oneshot::channel();
        cmd_tx.send(Cmd::Request { reply: reply_tx }).unwrap();

        // 2. Push an event. The loop MUST forward it before the
        //    request completes (which takes 10s of paused time).
        evt_tx.send("mid-flight").unwrap();

        // 3. The event should arrive immediately — tokio::time is paused,
        //    so "real" time can't even elapse. `recv` yielding a value
        //    proves the select! pumped events while the request is
        //    outstanding.
        let observed = tokio::time::timeout(std::time::Duration::from_millis(50), observed_rx.recv())
            .await
            .expect("event forwarded while request outstanding")
            .expect("channel open");
        assert_eq!(observed, "mid-flight");

        // Let the spawned request complete to keep the runtime clean.
        tokio::time::advance(std::time::Duration::from_secs(11)).await;
        let _ = reply_rx.await;
        drop(cmd_tx);
        drop(evt_tx);
        let _ = loop_handle.await;
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
            crate::adapters::InstanceKey::new_v4(),
            None,
            tx,
            Bootstrap::Resume(sid),
            dummy_permissions(),
            None,
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
