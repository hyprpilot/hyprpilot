//! Session registry shared across the RPC + Tauri command surfaces.
//!
//! Owns the live per-session actors spawned by `acp::runtime`.
//! Entries are keyed by agent id so that `session/submit` with an
//! optional `agent_id` can both resolve the active agent and reuse
//! the existing session for follow-up turns.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{json, Value};
use tauri::Emitter;
use tokio::sync::{broadcast, oneshot, Mutex};

use super::runtime::{start_session, SessionCommand, SessionEvent, SessionHandle};
use crate::config::{AgentConfig, AgentsConfig};
use crate::rpc::protocol::RpcError;
use crate::rpc::StatusBroadcast;

/// Capacity of the session-event broadcast. Slow subscribers drop
/// notifications; the webview resyncs from the next tick.
const EVENT_BROADCAST_CAPACITY: usize = 256;

#[derive(Debug)]
pub struct AcpSessions {
    pub(crate) config: AgentsConfig,
    #[allow(dead_code)]
    pub(crate) status: Arc<StatusBroadcast>,
    active: Mutex<HashMap<String, SessionHandle>>,
    events_tx: broadcast::Sender<SessionEvent>,
}

impl AcpSessions {
    #[must_use]
    pub fn new(config: AgentsConfig, status: Arc<StatusBroadcast>) -> Self {
        let (events_tx, _) = broadcast::channel(EVENT_BROADCAST_CAPACITY);
        Self {
            config,
            status,
            active: Mutex::new(HashMap::new()),
            events_tx,
        }
    }

    /// Broadcast receiver for every lifecycle + transcript event the
    /// active sessions emit. Tests subscribe directly; Tauri uses
    /// `spawn_tauri_event_bridge` instead.
    #[must_use]
    pub fn subscribe_events(&self) -> broadcast::Receiver<SessionEvent> {
        self.events_tx.subscribe()
    }

    /// Fan every `SessionEvent` out to the webview as an `acp:*`
    /// Tauri event. One subscriber task per daemon boot — call once
    /// from the Tauri `setup` closure.
    pub fn spawn_tauri_event_bridge(&self, app: tauri::AppHandle) {
        let mut rx = self.subscribe_events();
        tauri::async_runtime::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(evt) => emit_acp_event(&app, evt),
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(n, "acp events: subscriber lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
        });
    }

    /// Look up the configured entry for an agent id, defaulting to
    /// `[agent] default` when none is provided.
    fn resolve_cfg(&self, agent_id: Option<&str>) -> Result<AgentConfig, RpcError> {
        let wanted = agent_id
            .map(str::to_string)
            .or_else(|| self.config.agent.default.clone())
            .ok_or_else(|| RpcError::invalid_params("agent_id missing and no agent.default in config"))?;

        self.config
            .agents
            .iter()
            .find(|a| a.id == wanted)
            .cloned()
            .ok_or_else(|| RpcError::invalid_params(format!("agent '{wanted}' not found in [[agents]] registry")))
    }

    /// Submit a prompt. Spawns the agent if no live session exists;
    /// reuses the existing session otherwise. Returns the session id
    /// (or `null` if spawn is still in-flight).
    pub async fn submit(&self, text: &str, agent_id: Option<&str>) -> Result<Value, RpcError> {
        let cfg = self.resolve_cfg(agent_id)?;
        let handle_agent_id = cfg.id.clone();

        let cmd_tx = {
            let mut active = self.active.lock().await;
            if let Some(handle) = active.get(&handle_agent_id) {
                handle.cmd_tx.clone()
            } else {
                let handle = start_session(cfg.clone(), self.events_tx.clone());
                let tx = handle.cmd_tx.clone();
                active.insert(handle_agent_id.clone(), handle);
                tx
            }
        };

        let (reply_tx, reply_rx) = oneshot::channel();
        cmd_tx
            .send(SessionCommand::Prompt {
                text: text.to_string(),
                reply: reply_tx,
            })
            .map_err(|_| RpcError::internal_error("session actor closed before accepting prompt"))?;

        let session_id = {
            let active = self.active.lock().await;
            match active.get(&handle_agent_id) {
                Some(h) => h.current_session_id().await,
                None => None,
            }
        };

        tokio::spawn(async move {
            match reply_rx.await {
                Ok(Ok(())) => {}
                Ok(Err(err)) => tracing::warn!(%err, "acp::submit: prompt failed"),
                Err(_) => tracing::warn!("acp::submit: reply dropped before resolving"),
            }
        });

        Ok(json!({
            "accepted": true,
            "agent_id": handle_agent_id,
            "session_id": session_id,
        }))
    }

    /// Cancel the active turn on the addressed agent.
    pub async fn cancel(&self, agent_id: Option<&str>) -> Result<Value, RpcError> {
        let cfg = self.resolve_cfg(agent_id)?;
        let cmd_tx = {
            let active = self.active.lock().await;
            active.get(&cfg.id).map(|h| h.cmd_tx.clone())
        };

        let Some(cmd_tx) = cmd_tx else {
            return Ok(json!({ "cancelled": false, "reason": "no active session" }));
        };

        let (reply_tx, reply_rx) = oneshot::channel();
        if cmd_tx.send(SessionCommand::Cancel { reply: reply_tx }).is_err() {
            return Ok(json!({ "cancelled": false, "reason": "actor closed" }));
        }

        match reply_rx.await {
            Ok(Ok(())) => Ok(json!({ "cancelled": true })),
            Ok(Err(err)) => Ok(json!({ "cancelled": false, "reason": err })),
            Err(_) => Ok(json!({ "cancelled": false, "reason": "actor dropped reply" })),
        }
    }

    /// Snapshot of every live session.
    pub async fn info(&self) -> Result<Value, RpcError> {
        let active = self.active.lock().await;
        let mut sessions = Vec::with_capacity(active.len());
        for handle in active.values() {
            sessions.push(json!({
                "agent_id": handle.agent_id,
                "session_id": handle.current_session_id().await,
            }));
        }
        Ok(json!({ "sessions": sessions }))
    }

    /// Cleanup hook called from `daemon::shutdown` before `app.exit(0)`.
    /// Sends `Shutdown` to every active actor and drops the handles
    /// after the acks land (or immediately when the reply oneshot
    /// closes, whichever first).
    pub async fn shutdown(&self) {
        let handles: Vec<SessionHandle> = {
            let mut active = self.active.lock().await;
            active.drain().map(|(_, v)| v).collect()
        };
        tracing::info!(count = handles.len(), "acp::shutdown: draining sessions");
        for handle in handles {
            let (tx, rx) = oneshot::channel();
            let _ = handle.cmd_tx.send(SessionCommand::Shutdown { reply: tx });
            let _ = tokio::time::timeout(std::time::Duration::from_secs(2), rx).await;
        }
    }

    /// Enumerate configured agents for `agents_list`.
    #[must_use]
    pub fn list_agents(&self) -> Vec<Value> {
        self.config
            .agents
            .iter()
            .map(|a| {
                json!({
                    "id": a.id,
                    "provider": a.provider,
                    "is_default": self.config.agent.default.as_deref() == Some(a.id.as_str()),
                })
            })
            .collect()
    }
}

/// Route a runtime `SessionEvent` onto the corresponding `acp:*`
/// Tauri event. Separators stay `:` here; JSON-RPC wire uses `/`.
fn emit_acp_event(app: &tauri::AppHandle, evt: SessionEvent) {
    let name = match &evt {
        SessionEvent::State { .. } => "acp:session-state",
        SessionEvent::Transcript { .. } => "acp:transcript",
        SessionEvent::PermissionRequest { .. } => "acp:permission-request",
    };
    match serde_json::to_value(&evt) {
        Ok(v) => {
            if let Err(err) = app.emit(name, v) {
                tracing::warn!(%err, event = name, "failed to emit acp event");
            }
        }
        Err(err) => tracing::warn!(%err, event = name, "failed to serialize acp event"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn submit_without_default_is_invalid_params() {
        // Empty registry, no default.
        let cfg = AgentsConfig::default();
        let sessions = AcpSessions::new(cfg, Arc::new(StatusBroadcast::new(true)));
        let err = sessions.submit("hi", None).await.expect_err("must fail");
        assert_eq!(err.code, -32602);
    }

    #[tokio::test]
    async fn info_empty_when_nothing_spawned() {
        let cfg = AgentsConfig::default();
        let sessions = AcpSessions::new(cfg, Arc::new(StatusBroadcast::new(true)));
        let v = sessions.info().await.expect("ok");
        assert_eq!(v["sessions"], json!([]));
    }

    #[tokio::test]
    async fn cancel_unknown_agent_reports_missing_session() {
        let cfg = AgentsConfig::default();
        let sessions = AcpSessions::new(cfg, Arc::new(StatusBroadcast::new(true)));
        let err = sessions.cancel(Some("ghost")).await.expect_err("must fail");
        assert_eq!(err.code, -32602, "unknown agent id is invalid_params");
    }
}
