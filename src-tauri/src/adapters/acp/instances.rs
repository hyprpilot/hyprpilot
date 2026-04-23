//! Instance registry shared across the RPC + Tauri command surfaces.
//!
//! Owns the live per-instance actors spawned by `acp::runtime`.
//! Entries are keyed by `InstanceKey` (`agent_id` + optional
//! `profile_id`) so `session/submit` with an optional `profile_id`
//! keeps distinct instances per profile (a follow-up prompt with the
//! same `(agent_id, profile_id)` pair reuses the live actor; a
//! different profile against the same agent spawns its own).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use agent_client_protocol::schema::{ListSessionsResponse, SessionId};
use serde_json::{json, Value};
use tauri::Emitter;
use tokio::sync::{broadcast, oneshot, Mutex};

use super::instance::AcpInstance;
use super::resolve::ResolvedInstance;
use super::runtime::{start_instance, Bootstrap, InstanceCommand, InstanceEvent};
use crate::config::Config;
use crate::rpc::protocol::RpcError;
use crate::rpc::StatusBroadcast;

/// Capacity of the instance-event broadcast. Slow subscribers drop
/// notifications; the webview resyncs from the next tick.
const EVENT_BROADCAST_CAPACITY: usize = 256;

/// Registry key. `profile_id` is `None` for bare-agent resolutions
/// (no `[agent] default_profile` and no explicit `profile_id` on the
/// call). Two calls with the same `agent_id` but distinct profiles
/// get distinct actors — switching profile mid-instance changes the
/// system prompt and/or model, which are baked in at spawn time.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct InstanceKey {
    pub agent_id: String,
    pub profile_id: Option<String>,
}

impl InstanceKey {
    /// Stable string projection — the `instance_id` field on Tauri
    /// events + the log-span discriminator. `agent_id` alone when
    /// `profile_id` is `None`; `agent_id:profile_id` otherwise.
    #[must_use]
    pub fn as_string(&self) -> String {
        match &self.profile_id {
            Some(p) => format!("{}:{}", self.agent_id, p),
            None => self.agent_id.clone(),
        }
    }
}

#[derive(Debug)]
pub struct AcpInstances {
    pub(crate) config: Config,
    #[allow(dead_code)]
    pub(crate) status: Arc<StatusBroadcast>,
    active: Mutex<HashMap<InstanceKey, AcpInstance>>,
    events_tx: broadcast::Sender<InstanceEvent>,
}

impl AcpInstances {
    #[must_use]
    pub fn new(config: Config, status: Arc<StatusBroadcast>) -> Self {
        let (events_tx, _) = broadcast::channel(EVENT_BROADCAST_CAPACITY);
        Self {
            config,
            status,
            active: Mutex::new(HashMap::new()),
            events_tx,
        }
    }

    /// Broadcast receiver for every lifecycle + transcript event the
    /// active instances emit. Tests subscribe directly; Tauri uses
    /// `spawn_tauri_event_bridge` instead.
    #[must_use]
    pub fn subscribe_events(&self) -> broadcast::Receiver<InstanceEvent> {
        self.events_tx.subscribe()
    }

    /// Fan every `InstanceEvent` out to the webview as an `acp:*`
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

    /// Resolve a `(agent_id?, profile_id?)` pair. When both are
    /// omitted, falls back through `[agent] default_profile` and
    /// finally to `[agent] default`. Explicit `agent_id` overrides
    /// whatever agent the resolved profile names (same profile, new
    /// agent spawn).
    fn resolve(&self, agent_id: Option<&str>, profile_id: Option<&str>) -> Result<ResolvedInstance, RpcError> {
        let mut resolved = ResolvedInstance::from_config(&self.config, profile_id)
            .map_err(|e| RpcError::invalid_params(format!("{e:#}")))?;

        if let Some(wanted) = agent_id {
            let agent = self
                .config
                .agents
                .agents
                .iter()
                .find(|a| a.id == wanted)
                .cloned()
                .ok_or_else(|| {
                    RpcError::invalid_params(format!("agent '{wanted}' not found in [[agents]] registry"))
                })?;
            if resolved.model.is_none() || resolved.agent.id != agent.id {
                resolved.model = resolved.model.or_else(|| agent.model.clone());
            }
            resolved.agent = agent;
        }

        if resolved.agent.id.is_empty() {
            return Err(RpcError::invalid_params(
                "no agent resolved — add a [[agents]] entry or pass agent_id / profile_id",
            ));
        }

        Ok(resolved)
    }

    /// Submit a prompt. Spawns the agent if no live instance exists
    /// for this `(agent, profile)` pair; reuses the existing instance
    /// otherwise.
    pub async fn submit(
        &self,
        text: &str,
        agent_id: Option<&str>,
        profile_id: Option<&str>,
    ) -> Result<Value, RpcError> {
        let resolved = self.resolve(agent_id, profile_id)?;
        let key = InstanceKey {
            agent_id: resolved.agent.id.clone(),
            profile_id: resolved.profile_id.clone(),
        };

        tracing::info!(
            agent = %resolved.agent.id,
            profile = ?resolved.profile_id,
            model = ?resolved.model,
            has_prompt = resolved.system_prompt.is_some(),
            "acp::submit: resolved instance"
        );

        let cmd_tx = {
            let mut active = self.active.lock().await;
            if let Some(handle) = active.get(&key) {
                handle.cmd_tx.clone()
            } else {
                let instance = start_instance(
                    resolved.clone(),
                    key.as_string(),
                    self.events_tx.clone(),
                    Bootstrap::Fresh,
                );
                let tx = instance.cmd_tx.clone();
                active.insert(key.clone(), instance);
                tx
            }
        };

        let (reply_tx, reply_rx) = oneshot::channel();
        cmd_tx
            .send(InstanceCommand::Prompt {
                text: text.to_string(),
                reply: reply_tx,
            })
            .map_err(|_| RpcError::internal_error("instance actor closed before accepting prompt"))?;

        let session_id = {
            let active = self.active.lock().await;
            match active.get(&key) {
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
            "agent_id": key.agent_id,
            "profile_id": key.profile_id,
            "instance_id": key.as_string(),
            "session_id": session_id,
        }))
    }

    /// Cancel the active turn on the addressed agent.
    pub async fn cancel(&self, agent_id: Option<&str>) -> Result<Value, RpcError> {
        let resolved = self.resolve(agent_id, None)?;
        let cmd_tx = {
            let active = self.active.lock().await;
            active
                .iter()
                .find(|(k, _)| k.agent_id == resolved.agent.id)
                .map(|(_, h)| h.cmd_tx.clone())
        };

        let Some(cmd_tx) = cmd_tx else {
            return Ok(json!({ "cancelled": false, "reason": "no active instance" }));
        };

        let (reply_tx, reply_rx) = oneshot::channel();
        if cmd_tx.send(InstanceCommand::Cancel { reply: reply_tx }).is_err() {
            return Ok(json!({ "cancelled": false, "reason": "actor closed" }));
        }

        match reply_rx.await {
            Ok(Ok(())) => Ok(json!({ "cancelled": true })),
            Ok(Err(err)) => Ok(json!({ "cancelled": false, "reason": err })),
            Err(_) => Ok(json!({ "cancelled": false, "reason": "actor dropped reply" })),
        }
    }

    /// Snapshot of every live instance.
    pub async fn info(&self) -> Result<Value, RpcError> {
        let active = self.active.lock().await;
        let mut sessions = Vec::with_capacity(active.len());
        for (key, handle) in active.iter() {
            sessions.push(json!({
                "agent_id": handle.agent_id,
                "profile_id": key.profile_id,
                "instance_id": key.as_string(),
                "session_id": handle.current_session_id().await,
            }));
        }
        Ok(json!({ "sessions": sessions }))
    }

    /// Ask the agent for its persisted session index. Reuses the live
    /// actor when `(agent_id, profile_id)` is already bootstrapped;
    /// otherwise spawns an ephemeral `ListOnly` actor, issues the
    /// query, and tears it down. Ephemeral actors are never inserted
    /// into the registry — they exist for exactly one roundtrip.
    pub async fn list(
        &self,
        agent_id: Option<&str>,
        profile_id: Option<&str>,
        cwd: Option<PathBuf>,
    ) -> Result<ListSessionsResponse, RpcError> {
        let resolved = self.resolve(agent_id, profile_id)?;
        let key = InstanceKey {
            agent_id: resolved.agent.id.clone(),
            profile_id: resolved.profile_id.clone(),
        };

        let (cmd_tx, ephemeral) = {
            let active = self.active.lock().await;
            if let Some(handle) = active.get(&key) {
                (handle.cmd_tx.clone(), None)
            } else {
                let instance = start_instance(resolved, key.as_string(), self.events_tx.clone(), Bootstrap::ListOnly);
                let tx = instance.cmd_tx.clone();
                (tx, Some(instance))
            }
        };

        let (reply_tx, reply_rx) = oneshot::channel();
        cmd_tx
            .send(InstanceCommand::ListSessions { cwd, reply: reply_tx })
            .map_err(|_| RpcError::internal_error("instance actor closed before accepting list request"))?;

        let response = reply_rx
            .await
            .map_err(|_| RpcError::internal_error("instance actor dropped list reply"))?
            .map_err(|err| RpcError::internal_error(format!("session/list failed: {err}")));

        if let Some(handle) = ephemeral {
            let (tx, rx) = oneshot::channel();
            let _ = handle.cmd_tx.send(InstanceCommand::Shutdown { reply: tx });
            let _ = tokio::time::timeout(std::time::Duration::from_secs(2), rx).await;
        }

        response
    }

    /// Resume a persisted session. Tears down the live actor for
    /// `(agent_id, profile_id)` if present, then spawns a fresh actor
    /// with `Bootstrap::Resume(session_id)`. Replay updates stream
    /// through the standard `acp:transcript` event path.
    pub async fn load(
        &self,
        agent_id: Option<&str>,
        profile_id: Option<&str>,
        session_id: String,
    ) -> Result<(), RpcError> {
        let resolved = self.resolve(agent_id, profile_id)?;
        let key = InstanceKey {
            agent_id: resolved.agent.id.clone(),
            profile_id: resolved.profile_id.clone(),
        };

        let existing = {
            let mut active = self.active.lock().await;
            active.remove(&key)
        };
        if let Some(handle) = existing {
            let (tx, rx) = oneshot::channel();
            let _ = handle.cmd_tx.send(InstanceCommand::Shutdown { reply: tx });
            let _ = tokio::time::timeout(std::time::Duration::from_secs(2), rx).await;
        }

        let instance = start_instance(
            resolved,
            key.as_string(),
            self.events_tx.clone(),
            Bootstrap::Resume(SessionId::new(session_id)),
        );
        let mut active = self.active.lock().await;
        active.insert(key, instance);
        Ok(())
    }

    /// Cleanup hook called from `daemon::shutdown` before `app.exit(0)`.
    /// Sends `Shutdown` to every active actor and drops the handles
    /// after the acks land (or immediately when the reply oneshot
    /// closes, whichever first).
    pub async fn shutdown(&self) {
        let instances: Vec<AcpInstance> = {
            let mut active = self.active.lock().await;
            active.drain().map(|(_, v)| v).collect()
        };
        tracing::info!(count = instances.len(), "acp::shutdown: draining instances");
        for instance in instances {
            let (tx, rx) = oneshot::channel();
            let _ = instance.cmd_tx.send(InstanceCommand::Shutdown { reply: tx });
            let _ = tokio::time::timeout(std::time::Duration::from_secs(2), rx).await;
        }
    }

    /// Enumerate configured agents for `agents_list`.
    #[must_use]
    pub fn list_agents(&self) -> Vec<Value> {
        self.config
            .agents
            .agents
            .iter()
            .map(|a| {
                json!({
                    "id": a.id,
                    "provider": a.provider,
                    "is_default": self.config.agents.agent.default.as_deref() == Some(a.id.as_str()),
                })
            })
            .collect()
    }

    /// Enumerate configured profiles for `config/profiles` +
    /// `profiles_list`. `has_prompt` is `true` when either
    /// `system_prompt` is inline or a `system_prompt_file` is set —
    /// the file contents are not exposed here, matching the shape
    /// the chat UI needs (does this profile carry a custom prompt).
    pub fn list_profiles(&self) -> Vec<Value> {
        let default_profile = self.config.agents.agent.default_profile.as_deref();
        self.config
            .profiles
            .iter()
            .map(|p| {
                json!({
                    "id": p.id,
                    "agent": p.agent,
                    "model": p.model,
                    "has_prompt": p.system_prompt.is_some() || p.system_prompt_file.is_some(),
                    "is_default": default_profile == Some(p.id.as_str()),
                })
            })
            .collect()
    }
}

/// Route a runtime `InstanceEvent` onto the corresponding `acp:*`
/// Tauri event. Separators stay `:` here; JSON-RPC wire uses `/`.
fn emit_acp_event(app: &tauri::AppHandle, evt: InstanceEvent) {
    let name = match &evt {
        InstanceEvent::State { .. } => "acp:instance-state",
        InstanceEvent::Transcript { .. } => "acp:transcript",
        InstanceEvent::PermissionRequest { .. } => "acp:permission-request",
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
        let instances = AcpInstances::new(Config::default(), Arc::new(StatusBroadcast::new(true)));
        let err = instances.submit("hi", None, None).await.expect_err("must fail");
        assert_eq!(err.code, -32602);
    }

    #[tokio::test]
    async fn info_empty_when_nothing_spawned() {
        let instances = AcpInstances::new(Config::default(), Arc::new(StatusBroadcast::new(true)));
        let v = instances.info().await.expect("ok");
        assert_eq!(v["sessions"], json!([]));
    }

    #[tokio::test]
    async fn cancel_unknown_agent_reports_missing_session() {
        let instances = AcpInstances::new(Config::default(), Arc::new(StatusBroadcast::new(true)));
        let err = instances.cancel(Some("ghost")).await.expect_err("must fail");
        assert_eq!(err.code, -32602, "unknown agent id is invalid_params");
    }

    #[tokio::test]
    async fn resolve_honors_explicit_profile_id() {
        let cfg: Config = toml::from_str(
            r#"
[agent]
default = "claude-code"
default_profile = "ask"

[[agents]]
id = "claude-code"
provider = "acp-claude-code"
model = "claude-sonnet-4-5"

[[profiles]]
id = "ask"
agent = "claude-code"

[[profiles]]
id = "strict"
agent = "claude-code"
model = "claude-opus-4-5"
system_prompt = "be terse"
"#,
        )
        .expect("fixture parses");
        let instances = AcpInstances::new(cfg, Arc::new(StatusBroadcast::new(true)));

        let resolved = instances.resolve(None, Some("strict")).expect("strict resolves");
        assert_eq!(resolved.agent.id, "claude-code");
        assert_eq!(resolved.profile_id.as_deref(), Some("strict"));
        assert_eq!(resolved.model.as_deref(), Some("claude-opus-4-5"));
        assert_eq!(resolved.system_prompt.as_deref(), Some("be terse"));

        let resolved = instances.resolve(None, None).expect("default profile resolves");
        assert_eq!(resolved.profile_id.as_deref(), Some("ask"));
        assert_eq!(resolved.model.as_deref(), Some("claude-sonnet-4-5"));
        assert!(resolved.system_prompt.is_none());
    }

    #[tokio::test]
    async fn list_profiles_returns_configured_entries() {
        let cfg: Config = toml::from_str(
            r#"
[agent]
default_profile = "ask"

[[agents]]
id = "claude-code"
provider = "acp-claude-code"

[[profiles]]
id = "ask"
agent = "claude-code"

[[profiles]]
id = "strict"
agent = "claude-code"
system_prompt = "be terse"
"#,
        )
        .expect("parses");
        let instances = AcpInstances::new(cfg, Arc::new(StatusBroadcast::new(true)));
        let out = instances.list_profiles();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0]["id"], "ask");
        assert_eq!(out[0]["is_default"], true);
        assert_eq!(out[0]["has_prompt"], false);
        assert_eq!(out[1]["id"], "strict");
        assert_eq!(out[1]["is_default"], false);
        assert_eq!(out[1]["has_prompt"], true);
    }
}
