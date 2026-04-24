//! Instance registry shared across the RPC + Tauri command surfaces.
//!
//! Owns the live per-instance actors spawned by `acp::runtime`.
//! Entries are keyed by a UUID (`InstanceKey`) — a single agent/profile
//! pair can therefore back N concurrent instances, addressed by their
//! distinct ids. `agent_id` / `profile_id` are instance metadata, not
//! identity.
//!
//! Addressing:
//!   - `submit(text, Some(id), ...)`   — route to that UUID. If it
//!     doesn't exist yet, spawn with that id (adopt-on-first-sight).
//!   - `submit(text, None, ...)`       — mint a fresh UUID and spawn
//!     a new instance for the resolved `(agent, profile)`.
//!
//! Client-supplied UUIDs let the webview push the user's turn into
//! its local store BEFORE the RPC round-trip completes (the key is
//! known up-front), closing the seq race where agent responses landed
//! with lower seq than the user turn.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use agent_client_protocol::schema::{ListSessionsResponse, SessionId};
use serde_json::{json, Value};
use tauri::Emitter;
use tokio::sync::{broadcast, mpsc, oneshot, Mutex};
use uuid::Uuid;

use super::instance::AcpInstance;
use super::resolve::ResolvedInstance;
use super::runtime::{start_instance, Bootstrap, InstanceCommand, InstanceEvent};
use crate::adapters::permission::{DefaultPermissionController, PermissionController};
use crate::config::{Config, ProfileConfig};
use crate::rpc::protocol::RpcError;
use crate::rpc::StatusBroadcast;

/// Capacity of the instance-event broadcast. Slow subscribers drop
/// notifications; the webview resyncs from the next tick.
const EVENT_BROADCAST_CAPACITY: usize = 256;

/// Registry key. A UUID per live instance — collisions across twin
/// profiles are impossible by construction. Wire shape is the v4
/// hyphenated string (`Uuid::fmt`); the UI treats it as opaque.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub struct InstanceKey(pub Uuid);

impl InstanceKey {
    #[must_use]
    pub fn new_v4() -> Self {
        Self(Uuid::new_v4())
    }

    /// Parse a wire string (hyphenated v4). Surfaces as
    /// `-32602 invalid_params` when a client sends a malformed id.
    pub fn parse(s: &str) -> Result<Self, RpcError> {
        s.parse::<Uuid>()
            .map(Self)
            .map_err(|e| RpcError::invalid_params(format!("invalid instance_id '{s}': {e}")))
    }

    #[must_use]
    pub fn as_string(&self) -> String {
        self.0.to_string()
    }
}

impl std::fmt::Display for InstanceKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

pub struct AcpInstances {
    pub(crate) config: Config,
    #[allow(dead_code)]
    pub(crate) status: Arc<StatusBroadcast>,
    active: Mutex<HashMap<InstanceKey, AcpInstance>>,
    events_tx: broadcast::Sender<InstanceEvent>,
    permissions: Arc<dyn PermissionController>,
}

impl std::fmt::Debug for AcpInstances {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AcpInstances")
            .field("config", &self.config)
            .field("status", &self.status)
            .field("active", &self.active)
            .finish_non_exhaustive()
    }
}

impl AcpInstances {
    #[must_use]
    pub fn new(config: Config, status: Arc<StatusBroadcast>) -> Self {
        Self::with_permissions(
            config,
            status,
            Arc::new(DefaultPermissionController::new()) as Arc<dyn PermissionController>,
        )
    }

    #[must_use]
    pub fn with_permissions(
        config: Config,
        status: Arc<StatusBroadcast>,
        permissions: Arc<dyn PermissionController>,
    ) -> Self {
        let (events_tx, _) = broadcast::channel(EVENT_BROADCAST_CAPACITY);
        Self {
            config,
            status,
            active: Mutex::new(HashMap::new()),
            events_tx,
            permissions,
        }
    }

    /// Profile config lookup by id — used when spawning an actor so
    /// the runtime carries the full allowlist definition, not just a
    /// profile id.
    fn profile_by_id(&self, profile_id: Option<&str>) -> Option<ProfileConfig> {
        let id = profile_id?;
        self.config.profiles.iter().find(|p| p.id == id).cloned()
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

    /// Spawn-or-reuse for a given `InstanceKey`. Caller supplies the
    /// key (client-generated UUID for new instances; the existing key
    /// for follow-ups). `Bootstrap::Fresh` reuses a live instance at
    /// this key; `Resume(id)` tears any existing live instance down
    /// and replaces it with a session-load actor; `ListOnly` spawns
    /// an init-only actor and registers it (callers wanting truly
    /// ephemeral ListOnly actors construct them inline in `list` with
    /// a manual Shutdown).
    pub async fn ensure(
        &self,
        key: InstanceKey,
        resolved: ResolvedInstance,
        bootstrap: Bootstrap,
    ) -> Result<InstanceKey, RpcError> {
        let replace_existing = matches!(bootstrap, Bootstrap::Resume(_));
        let mut active = self.active.lock().await;

        if replace_existing {
            if let Some(existing) = active.remove(&key) {
                let (tx, rx) = oneshot::channel();
                let _ = existing.cmd_tx.send(InstanceCommand::Shutdown { reply: tx });
                drop(active);
                let _ = tokio::time::timeout(std::time::Duration::from_secs(2), rx).await;
                active = self.active.lock().await;
            }
        } else if active.contains_key(&key) {
            return Ok(key);
        }

        let profile = self.profile_by_id(resolved.profile_id.as_deref());
        let profile_id = resolved.profile_id.clone();
        let instance = start_instance(
            resolved,
            key.as_string(),
            profile_id,
            self.events_tx.clone(),
            bootstrap,
            self.permissions.clone(),
            profile,
        );
        active.insert(key, instance);
        Ok(key)
    }

    async fn cmd_tx_for(&self, key: &InstanceKey) -> Option<mpsc::UnboundedSender<InstanceCommand>> {
        let active = self.active.lock().await;
        active.get(key).map(|h| h.cmd_tx.clone())
    }

    /// Submit a prompt. When `instance_id` is provided, routes to
    /// (or adopts) that UUID; otherwise mints a fresh key and spawns
    /// a new instance against the resolved `(agent, profile)`.
    /// Multiple instances of the same `(agent, profile)` can coexist
    /// as long as they carry distinct UUIDs.
    pub async fn submit(
        &self,
        text: &str,
        instance_id: Option<&str>,
        agent_id: Option<&str>,
        profile_id: Option<&str>,
    ) -> Result<Value, RpcError> {
        let resolved = self.resolve(agent_id, profile_id)?;

        let key = match instance_id {
            Some(s) => InstanceKey::parse(s)?,
            None => InstanceKey::new_v4(),
        };

        tracing::info!(
            instance = %key,
            agent = %resolved.agent.id,
            profile = ?resolved.profile_id,
            model = ?resolved.model,
            has_prompt = resolved.system_prompt.is_some(),
            "acp::submit: resolved instance"
        );

        let resolved_agent_id = resolved.agent.id.clone();
        let resolved_profile_id = resolved.profile_id.clone();

        let key = self.ensure(key, resolved, Bootstrap::Fresh).await?;
        let cmd_tx = self
            .cmd_tx_for(&key)
            .await
            .ok_or_else(|| RpcError::internal_error("instance actor vanished before accepting prompt"))?;

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
            "agent_id": resolved_agent_id,
            "profile_id": resolved_profile_id,
            "instance_id": key.as_string(),
            "session_id": session_id,
        }))
    }

    /// Cancel the active turn. `instance_id` addresses a specific
    /// live instance; when omitted, falls back to the first live
    /// instance matching `agent_id`.
    pub async fn cancel(&self, instance_id: Option<&str>, agent_id: Option<&str>) -> Result<Value, RpcError> {
        let cmd_tx = if let Some(id) = instance_id {
            let key = InstanceKey::parse(id)?;
            self.cmd_tx_for(&key).await
        } else {
            let resolved = self.resolve(agent_id, None)?;
            let active = self.active.lock().await;
            active
                .values()
                .find(|h| h.agent_id == resolved.agent.id)
                .map(|h| h.cmd_tx.clone())
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
        let mut instances = Vec::with_capacity(active.len());
        for (key, handle) in active.iter() {
            instances.push(json!({
                "agent_id": handle.agent_id,
                "profile_id": handle.profile_id,
                "instance_id": key.as_string(),
                "session_id": handle.current_session_id().await,
            }));
        }
        Ok(json!({ "instances": instances }))
    }

    /// Ask the agent for its persisted session index. When
    /// `instance_id` is provided and live, reuses that actor;
    /// otherwise spawns an ephemeral `ListOnly` actor, issues the
    /// query, and tears it down. Ephemeral actors are never inserted
    /// into the registry — they exist for exactly one roundtrip.
    pub async fn list(
        &self,
        instance_id: Option<&str>,
        agent_id: Option<&str>,
        profile_id: Option<&str>,
        cwd: Option<PathBuf>,
    ) -> Result<ListSessionsResponse, RpcError> {
        let key = match instance_id {
            Some(s) => Some(InstanceKey::parse(s)?),
            None => None,
        };

        let (cmd_tx, ephemeral) = {
            let active = self.active.lock().await;
            let live = key.and_then(|k| active.get(&k).map(|h| h.cmd_tx.clone()));
            if let Some(tx) = live {
                (tx, None)
            } else {
                let resolved = self.resolve(agent_id, profile_id)?;
                let ephemeral_key = key.unwrap_or_else(InstanceKey::new_v4);
                let profile = self.profile_by_id(resolved.profile_id.as_deref());
                let profile_id_for_instance = resolved.profile_id.clone();
                let instance = start_instance(
                    resolved,
                    ephemeral_key.as_string(),
                    profile_id_for_instance,
                    self.events_tx.clone(),
                    Bootstrap::ListOnly,
                    self.permissions.clone(),
                    profile,
                );
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

    /// Resume a persisted session. `instance_id` addresses the live
    /// (or new) instance to bind the loaded session into — when
    /// omitted, mints a fresh key. Tears down the live actor at that
    /// key if present, then spawns with `Bootstrap::Resume(session_id)`.
    pub async fn load(
        &self,
        instance_id: Option<&str>,
        agent_id: Option<&str>,
        profile_id: Option<&str>,
        session_id: String,
    ) -> Result<(), RpcError> {
        let key = match instance_id {
            Some(s) => InstanceKey::parse(s)?,
            None => InstanceKey::new_v4(),
        };
        let resolved = self.resolve(agent_id, profile_id)?;
        self.ensure(key, resolved, Bootstrap::Resume(SessionId::new(session_id)))
            .await?;
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
/// Tauri event. Projects the ACP-side variant onto
/// `adapters::InstanceEvent` first — the generic vocabulary is the
/// wire shape the webview sees. Separators stay `:` here; JSON-RPC
/// wire uses `/`.
fn emit_acp_event(app: &tauri::AppHandle, evt: InstanceEvent) {
    let generic: crate::adapters::InstanceEvent = evt.into();
    let name = match &generic {
        crate::adapters::InstanceEvent::State { .. } => "acp:instance-state",
        crate::adapters::InstanceEvent::Transcript { .. } => "acp:transcript",
        crate::adapters::InstanceEvent::PermissionRequest { .. } => "acp:permission-request",
    };
    match serde_json::to_value(&generic) {
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
        let err = instances.submit("hi", None, None, None).await.expect_err("must fail");
        assert_eq!(err.code, -32602);
    }

    #[tokio::test]
    async fn info_empty_when_nothing_spawned() {
        let instances = AcpInstances::new(Config::default(), Arc::new(StatusBroadcast::new(true)));
        let v = instances.info().await.expect("ok");
        assert_eq!(v["instances"], json!([]));
    }

    #[tokio::test]
    async fn cancel_unknown_agent_reports_missing_session() {
        let instances = AcpInstances::new(Config::default(), Arc::new(StatusBroadcast::new(true)));
        let err = instances.cancel(None, Some("ghost")).await.expect_err("must fail");
        assert_eq!(err.code, -32602, "unknown agent id is invalid_params");
    }

    #[tokio::test]
    async fn cancel_invalid_instance_id_is_invalid_params() {
        let instances = AcpInstances::new(Config::default(), Arc::new(StatusBroadcast::new(true)));
        let err = instances.cancel(Some("not-a-uuid"), None).await.expect_err("must fail");
        assert_eq!(err.code, -32602, "malformed instance_id is invalid_params");
    }

    #[tokio::test]
    async fn instance_key_roundtrips_v4_string() {
        let k = InstanceKey::new_v4();
        let s = k.as_string();
        let parsed = InstanceKey::parse(&s).expect("parse clean");
        assert_eq!(k, parsed);
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
