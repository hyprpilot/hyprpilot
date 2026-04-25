//! ACP adapter facade: composes `AdapterRegistry<AcpInstance>` and
//! carries the ACP-specific glue (profile resolution, vendor
//! `(command, args)` spawn, permission controller). The registry is
//! the generic piece; everything here is the ACP translation layer.
//!
//! Addressing:
//!   - `submit(text, Some(id), ...)` — route to that UUID. If it
//!     doesn't exist yet, spawn with that id (adopt-on-first-sight).
//!   - `submit(text, None, ...)`     — mint a fresh UUID and spawn
//!     a new instance for the resolved `(agent, profile)`.
//!
//! Client-supplied UUIDs let the webview push the user's turn into
//! its local store BEFORE the RPC round-trip completes (the key is
//! known up-front), closing the seq race where agent responses landed
//! with lower seq than the user turn.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, OnceLock, RwLock};

use agent_client_protocol::schema::{ListSessionsResponse, SessionId};
use async_trait::async_trait;
use serde_json::{json, Value};
use tauri::Emitter;
use tokio::sync::{broadcast, mpsc, oneshot};

use super::instance::AcpInstance;
use super::runtime::{start_instance, Bootstrap, InstanceCommand, InstanceEvent};
use crate::adapters::instance::InstanceActor;
use crate::adapters::permission::{DefaultPermissionController, PermissionController};
use crate::adapters::profile::ResolvedInstance;
use crate::adapters::registry::AdapterRegistry;
use crate::adapters::{
    Adapter, AdapterError, AdapterId, AdapterResult, Capabilities, InstanceEventStream, InstanceInfo, InstanceKey,
    SpawnSpec, UserTurnInput,
};
use crate::config::{Config, ProfileConfig};
use crate::rpc::protocol::RpcError;
use crate::rpc::StatusBroadcast;

pub struct AcpAdapter {
    /// Shared config handle. Read-only at runtime — config is static
    /// after daemon start, restart-to-change is the model. Wrapped in
    /// an `RwLock` so the daemon can hand the same `Arc` to `RpcState`
    /// for read-only handlers (`config/profiles`) without re-cloning.
    pub(crate) config: Arc<RwLock<Config>>,
    #[allow(dead_code)]
    pub(crate) status: Arc<StatusBroadcast>,
    registry: Arc<AdapterRegistry<AcpInstance>>,
    permissions: Arc<dyn PermissionController>,
    /// Per-instance MCP enabled-list overrides. Keyed by `InstanceKey`,
    /// installed by `mcps/set` and read at spawn time (after restart)
    /// to compute the effective MCP set. Survives restart of the
    /// addressed instance (key preserved); cleared on shutdown.
    mcps_overrides: Arc<RwLock<HashMap<InstanceKey, Vec<String>>>>,
    /// Lazy-init channel the per-instance runtime actor publishes
    /// ACP-shape events onto. A dedicated task forwards each event
    /// through `mapping::From` onto the registry's generic broadcast.
    /// Spawning the bridge eagerly in the constructor would `tokio::spawn`
    /// before Tauri's runtime is alive ("no reactor running" panic on
    /// `daemon::run`); the `OnceLock` defers spawn to first access,
    /// which always lands inside the Tauri `.setup(...)` closure.
    runtime_events_bridge: OnceLock<broadcast::Sender<InstanceEvent>>,
    /// Instance ids with at least one in-flight turn. Driven by a
    /// background task subscribed to the registry's broadcast —
    /// `TurnStarted` adds, `TurnEnded` removes. Read by
    /// `daemon/shutdown`'s busy check.
    busy_instances: Arc<RwLock<HashSet<String>>>,
}

impl std::fmt::Debug for AcpAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AcpAdapter")
            .field("config", &self.config)
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

impl AcpAdapter {
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
        Self::with_shared_config(Arc::new(RwLock::new(config)), status, permissions)
    }

    /// Construct against an already-shared config handle. Used by the
    /// daemon so `RpcState.config` and `AcpAdapter.config` point at the
    /// same `RwLock<Config>` instance — readers locking briefly clone
    /// what they need.
    #[must_use]
    pub fn with_shared_config(
        config: Arc<RwLock<Config>>,
        status: Arc<StatusBroadcast>,
        permissions: Arc<dyn PermissionController>,
    ) -> Self {
        Self {
            config,
            status,
            registry: Arc::new(AdapterRegistry::new()),
            permissions,
            mcps_overrides: Arc::new(RwLock::new(HashMap::new())),
            runtime_events_bridge: OnceLock::new(),
            busy_instances: Arc::new(RwLock::new(HashSet::new())),
        }
    }

    /// Effective MCP enabled-list for an instance: per-instance
    /// override wins; otherwise the resolved profile's `mcps` field;
    /// otherwise `None` (meaning "all catalog entries enabled" — the
    /// default-empty-list semantics from CLAUDE.md).
    pub(crate) async fn effective_mcps_for(&self, key: InstanceKey) -> Option<Vec<String>> {
        if let Some(o) = self
            .mcps_overrides
            .read()
            .expect("mcps overrides lock poisoned")
            .get(&key)
            .cloned()
        {
            return Some(o);
        }
        let profile_id = self.registry.get(key).await.and_then(|h| h.profile_id.clone())?;
        let cfg = self.read_config();
        cfg.profiles
            .iter()
            .find(|p| p.id == profile_id)
            .and_then(|p| p.mcps.clone())
    }

    /// Install a per-instance MCP override. Replaces any existing
    /// override for `key`; subsequent `restart` reads it back when
    /// (re-)spawning the actor. Returns the previous override if any.
    pub(crate) fn set_mcps_override(&self, key: InstanceKey, enabled: Vec<String>) -> Option<Vec<String>> {
        let mut map = self.mcps_overrides.write().expect("mcps overrides lock poisoned");
        map.insert(key, enabled)
    }

    /// Drop a per-instance MCP override. Called from shutdown paths
    /// so a fresh-spawned instance reusing a recycled UUID doesn't
    /// inherit a stale override. (UUIDs are unique by construction
    /// today — this is defensive.)
    #[allow(dead_code)]
    pub(crate) fn clear_mcps_override(&self, key: InstanceKey) {
        let mut map = self.mcps_overrides.write().expect("mcps overrides lock poisoned");
        map.remove(&key);
    }

    /// Handle onto the shared config. Used by the daemon wiring to
    /// hand the same lock to `RpcState` so reads + writes stay
    /// coherent.
    #[must_use]
    pub fn shared_config(&self) -> Arc<RwLock<Config>> {
        self.config.clone()
    }

    /// Handle onto the shared permission controller. Used by the
    /// `permissions/*` RPC handlers to enumerate + resolve waiters
    /// against the same map the runtime registered them in.
    #[must_use]
    pub fn permissions(&self) -> Arc<dyn PermissionController> {
        self.permissions.clone()
    }

    /// Lazily spawn (and memoise) the ACP → generic events bridge
    /// task. Defers the `tokio::spawn` call until first access, which
    /// always happens after `daemon::run` enters the Tauri builder's
    /// `.setup(...)` closure (runtime alive). Subsequent calls return
    /// the same sender clone.
    ///
    /// Also kicks off the busy-tracker task on first access — it
    /// subscribes to the registry's generic broadcast and follows
    /// `TurnStarted` / `TurnEnded` to maintain `busy_instances`.
    fn runtime_bridge(&self) -> &broadcast::Sender<InstanceEvent> {
        self.runtime_events_bridge.get_or_init(|| {
            self.spawn_busy_tracker();
            Self::spawn_runtime_bridge_inner(self.registry.clone())
        })
    }

    /// Subscribe to the registry's generic broadcast and maintain the
    /// `busy_instances` set off `TurnStarted` / `TurnEnded`. Called
    /// once via the `runtime_bridge` `OnceLock` so the spawn lands
    /// inside the Tauri runtime context.
    fn spawn_busy_tracker(&self) {
        let mut rx = self.registry.subscribe();
        let busy = self.busy_instances.clone();
        tokio::spawn(async move {
            use crate::adapters::InstanceEvent;
            loop {
                match rx.recv().await {
                    Ok(InstanceEvent::TurnStarted { instance_id, .. }) => {
                        if let Ok(mut set) = busy.write() {
                            set.insert(instance_id);
                        }
                    }
                    Ok(InstanceEvent::TurnEnded { instance_id, .. }) => {
                        if let Ok(mut set) = busy.write() {
                            set.remove(&instance_id);
                        }
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(n, "acp busy tracker: lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
        });
    }

    /// Snapshot of every instance id currently mid-turn. Used by
    /// `daemon/shutdown` for the busy check.
    pub fn busy_instance_ids(&self) -> impl std::future::Future<Output = Vec<String>> + Send {
        let busy = self.busy_instances.clone();
        async move { busy.read().map(|set| set.iter().cloned().collect()).unwrap_or_default() }
    }

    /// Publish a `DaemonReloaded` event onto the registry's broadcast.
    /// Invoked by the `daemon/reload` RPC handler after the config +
    /// skills rescans complete.
    pub fn publish_daemon_reloaded(&self, profiles: usize, skills_count: usize, mcps_count: usize) {
        let _ = self
            .registry
            .events_tx()
            .send(crate::adapters::InstanceEvent::DaemonReloaded {
                profiles,
                skills_count,
                mcps_count,
            });
    }

    /// Test hook: mark an instance id as busy for the busy-check
    /// path without driving the runtime. Production paths drive this
    /// through the broadcast bridge instead.
    #[cfg(test)]
    pub fn test_mark_busy(&self, id: String) {
        if let Ok(mut set) = self.busy_instances.write() {
            set.insert(id);
        }
    }

    /// Profile config lookup by id — used when spawning an actor so
    /// the runtime carries the full allowlist definition, not just a
    /// profile id.
    fn profile_by_id(&self, profile_id: Option<&str>) -> Option<ProfileConfig> {
        let id = profile_id?;
        self.read_config().profiles.iter().find(|p| p.id == id).cloned()
    }

    /// Short-lived read guard helper. Callers drop before any `.await`
    /// — `std::sync::RwLock` isn't `Send` across suspension points.
    fn read_config(&self) -> std::sync::RwLockReadGuard<'_, Config> {
        self.config.read().expect("AcpAdapter config lock poisoned")
    }

    /// Broadcast receiver for every lifecycle + transcript event the
    /// active instances emit. Tests subscribe directly; Tauri uses
    /// `spawn_tauri_event_bridge` instead. Subscribers must handle
    /// `broadcast::error::RecvError::Lagged` — the channel drops
    /// messages silently otherwise.
    #[must_use]
    pub fn subscribe_events(&self) -> broadcast::Receiver<crate::adapters::InstanceEvent> {
        self.registry.subscribe()
    }

    /// Test-only publish handle. Production publishers go through the
    /// runtime → registry pipeline; integration tests reach for this
    /// to drive deterministic events through the broadcast without
    /// spawning a real ACP actor.
    #[cfg(test)]
    #[must_use]
    pub fn test_events_tx(&self) -> broadcast::Sender<crate::adapters::InstanceEvent> {
        self.registry.events_tx()
    }
}

impl AcpAdapter {
    /// Route a generic `InstanceEvent` onto the corresponding `acp:*`
    /// Tauri event. Projects the generic shape onto the Tauri naming
    /// convention (`:` separators). Keeps wire topics (`.`) vs Tauri
    /// event names (`:`) as distinct axes.
    ///
    /// Consumers must handle `broadcast::error::RecvError::Lagged`
    /// — the broadcast channel silently drops notifications
    /// otherwise.
    pub fn spawn_tauri_event_bridge(&self, app: tauri::AppHandle) {
        let mut rx = self.registry.subscribe();
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
        let cfg = self.read_config();
        let mut resolved =
            ResolvedInstance::from_config(&cfg, profile_id).map_err(|e| RpcError::invalid_params(format!("{e:#}")))?;

        if let Some(wanted) = agent_id {
            let agent = cfg
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
    async fn ensure(
        &self,
        key: InstanceKey,
        resolved: ResolvedInstance,
        bootstrap: Bootstrap,
    ) -> Result<InstanceKey, RpcError> {
        let replace_existing = matches!(bootstrap, Bootstrap::Resume(_));
        if !replace_existing && self.registry.get(key).await.is_some() {
            return Ok(key);
        }
        if replace_existing {
            let _ = self.registry.shutdown_one(key).await;
        }

        let profile = self.profile_by_id(resolved.profile_id.as_deref());
        let profile_id = resolved.profile_id.clone();
        let mcps_override = self
            .mcps_overrides
            .read()
            .expect("mcps overrides lock poisoned")
            .get(&key)
            .cloned();
        let instance = start_instance(
            resolved,
            key,
            profile_id,
            self.runtime_bridge().clone(),
            bootstrap,
            self.permissions.clone(),
            profile,
            mcps_override,
        );

        self.registry
            .insert(key, Arc::new(instance))
            .await
            .map_err(map_adapter_error_to_rpc)?;
        Ok(key)
    }

    async fn cmd_tx_for(&self, key: &InstanceKey) -> Option<mpsc::UnboundedSender<InstanceCommand>> {
        self.registry.get(*key).await.map(|h| h.cmd_tx.clone())
    }

    /// Submit a prompt with optional attachments. When `instance_id`
    /// is provided, routes to (or adopts) that UUID; otherwise mints
    /// a fresh key and spawns a new instance against the resolved
    /// `(agent, profile)`. Attachments project onto the wire as
    /// `ContentBlock::Resource` per `mapping::build_prompt_blocks`.
    pub async fn submit_prompt(
        &self,
        text: &str,
        attachments: &[crate::adapters::Attachment],
        instance_id: Option<&str>,
        agent_id: Option<&str>,
        profile_id: Option<&str>,
    ) -> Result<Value, RpcError> {
        let resolved = self.resolve(agent_id, profile_id)?;

        let key = match instance_id {
            Some(s) => InstanceKey::parse(s).map_err(map_adapter_error_to_rpc)?,
            None => InstanceKey::new_v4(),
        };

        tracing::info!(
            instance = %key,
            agent = %resolved.agent.id,
            profile = ?resolved.profile_id,
            model = ?resolved.model,
            has_prompt = resolved.system_prompt.is_some(),
            attachments = attachments.len(),
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
                attachments: attachments.to_vec(),
                reply: reply_tx,
            })
            .map_err(|_| RpcError::internal_error("instance actor closed before accepting prompt"))?;

        let session_id = match self.registry.get(key).await {
            Some(h) => h.current_session_id().await,
            None => None,
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
            "agentId": resolved_agent_id,
            "profileId": resolved_profile_id,
            "instanceId": key.as_string(),
            "sessionId": session_id,
        }))
    }

    /// Cancel the active turn. `instance_id` addresses a specific
    /// live instance; when omitted, falls back to the first live
    /// instance matching `agent_id`.
    pub async fn cancel_active(&self, instance_id: Option<&str>, agent_id: Option<&str>) -> Result<Value, RpcError> {
        let cmd_tx = if let Some(id) = instance_id {
            let key = InstanceKey::parse(id).map_err(map_adapter_error_to_rpc)?;
            self.cmd_tx_for(&key).await
        } else {
            let resolved = self.resolve(agent_id, None)?;
            let keys = self.registry.ordered_keys().await;
            let mut tx = None;
            for k in keys {
                if let Some(h) = self.registry.get(k).await {
                    if h.agent_id == resolved.agent.id {
                        tx = Some(h.cmd_tx.clone());
                        break;
                    }
                }
            }
            tx
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

    /// Snapshot of every live instance in the legacy `{ instances: [...] }`
    /// envelope. `Adapter::list` returns typed `InstanceInfo[]` for
    /// programmatic consumers.
    pub async fn info_json(&self) -> Result<Value, RpcError> {
        let snapshot = self.registry.list().await;
        let instances: Vec<_> = snapshot
            .into_iter()
            .map(|info| {
                json!({
                    "agentId": info.agent_id,
                    "profileId": info.profile_id,
                    "instanceId": info.id,
                    "sessionId": info.session_id,
                    "mode": info.mode,
                })
            })
            .collect();
        Ok(json!({ "instances": instances }))
    }

    /// Ask the agent for its persisted session index. When
    /// `instance_id` is provided and live, reuses that actor;
    /// otherwise spawns an ephemeral `ListOnly` actor, issues the
    /// query, and tears it down. Ephemeral actors are never inserted
    /// into the registry — they exist for exactly one roundtrip.
    pub async fn list_sessions(
        &self,
        instance_id: Option<&str>,
        agent_id: Option<&str>,
        profile_id: Option<&str>,
        cwd: Option<PathBuf>,
    ) -> Result<ListSessionsResponse, RpcError> {
        let key = match instance_id {
            Some(s) => Some(InstanceKey::parse(s).map_err(map_adapter_error_to_rpc)?),
            None => None,
        };

        let live_tx = match key {
            Some(k) => self.registry.get(k).await.map(|h| h.cmd_tx.clone()),
            None => None,
        };

        let (cmd_tx, ephemeral) = if let Some(tx) = live_tx {
            (tx, None)
        } else {
            let resolved = self.resolve(agent_id, profile_id)?;
            let ephemeral_key = key.unwrap_or_else(InstanceKey::new_v4);
            let profile = self.profile_by_id(resolved.profile_id.as_deref());
            let profile_id_for_instance = resolved.profile_id.clone();
            // Ephemeral list-only actor never reads MCPs; pass None.
            let instance = start_instance(
                resolved,
                ephemeral_key,
                profile_id_for_instance,
                self.runtime_bridge().clone(),
                Bootstrap::ListOnly,
                self.permissions.clone(),
                profile,
                None,
            );
            let tx = instance.cmd_tx.clone();
            (tx, Some(instance))
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
            handle.shutdown().await;
        }

        response
    }

    /// Resume a persisted session. `instance_id` addresses the live
    /// (or new) instance to bind the loaded session into — when
    /// omitted, mints a fresh key. Tears down the live actor at that
    /// key if present, then spawns with `Bootstrap::Resume(session_id)`.
    pub async fn load_session(
        &self,
        instance_id: Option<&str>,
        agent_id: Option<&str>,
        profile_id: Option<&str>,
        session_id: String,
    ) -> Result<(), RpcError> {
        let key = match instance_id {
            Some(s) => InstanceKey::parse(s).map_err(map_adapter_error_to_rpc)?,
            None => InstanceKey::new_v4(),
        };
        let resolved = self.resolve(agent_id, profile_id)?;
        self.ensure(key, resolved, Bootstrap::Resume(SessionId::new(session_id)))
            .await?;
        Ok(())
    }

    /// Cleanup hook called from `daemon::shutdown` before `app.exit(0)`.
    /// Sends `Shutdown` to every active actor and drops the handles
    /// after the acks land.
    pub async fn shutdown_all(&self) {
        let instances = self.registry.drain().await;
        tracing::info!(count = instances.len(), "acp::shutdown: draining instances");
        for instance in instances {
            instance.shutdown().await;
        }
        self.mcps_overrides
            .write()
            .expect("mcps overrides lock poisoned")
            .clear();
    }

    /// Spawn a fresh instance against the resolved `(agent, profile)`.
    /// `cwd` / `model` / `mode` overlay on top of the resolved config
    /// before spawn.
    pub async fn spawn_instance(&self, spec: SpawnSpec) -> Result<InstanceKey, RpcError> {
        let SpawnSpec {
            profile_id,
            agent_id,
            cwd,
            mode,
            model,
        } = spec;
        let mut resolved = self.resolve(agent_id.as_deref(), profile_id.as_deref())?;
        if let Some(c) = cwd {
            resolved.agent.cwd = Some(c);
        }
        if model.is_some() {
            resolved.model = model;
        }
        if mode.is_some() {
            resolved.mode = mode;
        }
        let key = InstanceKey::new_v4();
        self.ensure(key, resolved, Bootstrap::Fresh).await
    }

    /// Graceful shutdown of the instance at `key`, then an immediate
    /// spawn against the same resolved config under the same key and
    /// insertion-order slot. Preserves UUID identity so subscribers
    /// stay bound; preserves slot so auto-focus on next shutdown
    /// behaves consistently.
    pub async fn restart_instance(&self, key: InstanceKey) -> Result<InstanceKey, RpcError> {
        let existing = self
            .registry
            .get(key)
            .await
            .ok_or_else(|| RpcError::invalid_params(format!("instance '{key}' not found in registry")))?;
        let agent_id = existing.agent_id.clone();
        let profile_id = existing.profile_id.clone();
        let mode = existing.mode.clone();
        drop(existing);

        let slot = self
            .registry
            .drop_preserving_slot(key)
            .await
            .map_err(map_adapter_error_to_rpc)?;

        let mut resolved = self.resolve(Some(&agent_id), profile_id.as_deref())?;
        if mode.is_some() {
            resolved.mode = mode;
        }
        let profile = self.profile_by_id(resolved.profile_id.as_deref());
        let profile_id_for_instance = resolved.profile_id.clone();
        // Restart preserves the per-instance MCP override, so the
        // post-restart actor reads the same effective set.
        let mcps_override = self
            .mcps_overrides
            .read()
            .expect("mcps overrides lock poisoned")
            .get(&key)
            .cloned();
        let instance = start_instance(
            resolved,
            key,
            profile_id_for_instance,
            self.runtime_bridge().clone(),
            Bootstrap::Fresh,
            self.permissions.clone(),
            profile,
            mcps_override,
        );
        self.registry
            .insert_at_slot(slot, key, Arc::new(instance))
            .await
            .map_err(map_adapter_error_to_rpc)?;
        Ok(key)
    }

    /// Enumerate configured agents for `agents_list`.
    #[must_use]
    pub fn list_agents(&self) -> Vec<Value> {
        let cfg = self.read_config();
        let default_agent = cfg.agents.agent.default.as_deref();
        cfg.agents
            .agents
            .iter()
            .map(|a| {
                json!({
                    "id": a.id,
                    "provider": a.provider,
                    "binding": a.command,
                    "isDefault": default_agent == Some(a.id.as_str()),
                })
            })
            .collect()
    }

    /// Enumerate configured profiles for `config/profiles` +
    /// `profiles/list`. Wire shape: `{ id, agent, model, is_default }`.
    /// Caller (chat-shell picker) highlights `is_default`; `agent` +
    /// `model` disambiguate; `id` is the registry key.
    pub fn list_profiles(&self) -> Vec<Value> {
        let cfg = self.read_config();
        let default_profile = cfg.agents.agent.default_profile.as_deref();
        cfg.profiles
            .iter()
            .map(|p| {
                json!({
                    "id": p.id,
                    "agent": p.agent,
                    "model": p.model,
                    "isDefault": default_profile == Some(p.id.as_str()),
                })
            })
            .collect()
    }

    /// Shutdown a single instance and auto-focus the oldest survivor.
    /// Drops any per-instance MCP override so a UUID never inherits a
    /// stale override (defensive — UUIDs are unique by construction).
    pub async fn shutdown_instance(&self, key: InstanceKey) -> Result<InstanceKey, RpcError> {
        let key = self
            .registry
            .shutdown_one(key)
            .await
            .map_err(map_adapter_error_to_rpc)?;
        self.clear_mcps_override(key);
        Ok(key)
    }

    /// Designate the focused instance. Unknown id → `-32602 invalid_params`.
    pub async fn focus_instance(&self, key: InstanceKey) -> Result<InstanceKey, RpcError> {
        self.registry.focus(key).await.map_err(map_adapter_error_to_rpc)
    }

    /// Slash-commands cache (K-267 palette leaf, K-280 wire).
    /// `AcpInstance` will cache the `available_commands` SessionUpdate
    /// in K-251; until then this surfaces the same `-32603` the RPC
    /// handler does so the Tauri caller can render an empty / error
    /// state without inventing a fake-success path.
    pub async fn list_commands(&self, id: &str) -> Result<Vec<Value>, RpcError> {
        let _ = self.contains_instance(id).await?;
        Err(RpcError::internal_error("commands/list not implemented — ref K-251"))
    }

    /// Membership check used by `modes/*`, `models/*`, `commands/*`
    /// handlers to map a wire-supplied `instance_id` onto the live
    /// registry. `-32602` when the id is malformed or not in the
    /// registry — same failure mode both paths get.
    pub async fn contains_instance(&self, id: &str) -> Result<InstanceKey, RpcError> {
        let key = InstanceKey::parse(id).map_err(map_adapter_error_to_rpc)?;
        match self.registry.get(key).await {
            Some(_) => Ok(key),
            None => Err(RpcError::invalid_params(format!(
                "instance '{id}' not found in registry"
            ))),
        }
    }
}

// Runtime actors speak the ACP-internal `InstanceEvent` enum; the
// registry speaks the generic `adapters::InstanceEvent`. We bridge
// ACP → generic in a background task so both wire shapes stay
// self-documenting without the actor importing generic types. Owned
// by the adapter so the bridge task dies when the adapter drops.
impl AcpAdapter {
    /// Construct the ACP → generic bridge channel + task. Invoked
    /// once on first `runtime_bridge()` access via the `OnceLock`'s
    /// initialiser.
    fn spawn_runtime_bridge_inner(registry: Arc<AdapterRegistry<AcpInstance>>) -> broadcast::Sender<InstanceEvent> {
        let (runtime_tx, mut runtime_rx) = broadcast::channel::<InstanceEvent>(256);
        let out_tx = registry.events_tx();
        tokio::spawn(async move {
            loop {
                match runtime_rx.recv().await {
                    Ok(evt) => {
                        let generic: crate::adapters::InstanceEvent = evt.into();
                        let _ = out_tx.send(generic);
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(n, "acp runtime bridge: lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
        });
        runtime_tx
    }
}

#[async_trait]
impl Adapter for AcpAdapter {
    fn id(&self) -> AdapterId {
        AdapterId::Acp
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            load_session: true,
            list_sessions: true,
            permissions: true,
            terminals: true,
        }
    }

    async fn list(&self) -> Vec<InstanceInfo> {
        // Async session_id fill-in: generic registry snapshots are
        // sync (`InstanceActor::info` is a plain fn), so a live
        // session id that's still in-flight can show up as None.
        // Post-process here to block on the RwLock read.
        let base = self.registry.list().await;
        let mut out = Vec::with_capacity(base.len());
        for mut info in base {
            if info.session_id.is_none() {
                if let Ok(key) = InstanceKey::parse(&info.id) {
                    if let Some(handle) = self.registry.get(key).await {
                        info.session_id = handle.current_session_id().await;
                    }
                }
            }
            out.push(info);
        }
        out
    }

    async fn info_for(&self, key: InstanceKey) -> AdapterResult<InstanceInfo> {
        let mut info = self.registry.info_for(key).await?;
        if info.session_id.is_none() {
            if let Some(handle) = self.registry.get(key).await {
                info.session_id = handle.current_session_id().await;
            }
        }
        Ok(info)
    }

    async fn focused_id(&self) -> Option<InstanceKey> {
        self.registry.focused_id().await
    }

    async fn focus(&self, key: InstanceKey) -> AdapterResult<InstanceKey> {
        self.registry.focus(key).await
    }

    async fn shutdown_one(&self, key: InstanceKey) -> AdapterResult<InstanceKey> {
        self.registry.shutdown_one(key).await
    }

    async fn restart(&self, key: InstanceKey) -> AdapterResult<InstanceKey> {
        self.restart_instance(key).await.map_err(rpc_to_adapter)
    }

    fn subscribe(&self) -> InstanceEventStream {
        self.registry.subscribe()
    }

    async fn spawn(&self, spec: SpawnSpec) -> AdapterResult<InstanceKey> {
        self.spawn_instance(spec).await.map_err(rpc_to_adapter)
    }

    async fn submit(
        &self,
        input: UserTurnInput,
        instance_id: Option<&str>,
        agent_id: Option<&str>,
        profile_id: Option<&str>,
    ) -> AdapterResult<serde_json::Value> {
        let UserTurnInput::Prompt { text, attachments } = input;
        self.submit_prompt(&text, &attachments, instance_id, agent_id, profile_id)
            .await
            .map_err(rpc_to_adapter)
    }

    async fn cancel(&self, instance_id: Option<&str>, agent_id: Option<&str>) -> AdapterResult<serde_json::Value> {
        self.cancel_active(instance_id, agent_id).await.map_err(rpc_to_adapter)
    }

    async fn info(&self) -> AdapterResult<serde_json::Value> {
        self.info_json().await.map_err(rpc_to_adapter)
    }

    async fn shutdown(&self) {
        self.shutdown_all().await;
    }
}

fn map_adapter_error_to_rpc(err: AdapterError) -> RpcError {
    match err {
        AdapterError::InvalidRequest(m) => RpcError::invalid_params(m),
        AdapterError::Unsupported(m) => RpcError::method_not_found(&m),
        AdapterError::Backend(m) => RpcError::internal_error(m),
    }
}

fn rpc_to_adapter(err: RpcError) -> AdapterError {
    match err.code {
        RpcError::CODE_INVALID_PARAMS => AdapterError::InvalidRequest(err.message),
        RpcError::CODE_METHOD_NOT_FOUND => AdapterError::Unsupported(err.message),
        _ => AdapterError::Backend(err.message),
    }
}

/// Route a generic `InstanceEvent` onto the corresponding `acp:*`
/// Tauri event. Names follow the Tauri-side convention (`:`
/// separators); the dot-separated wire topic is accessible via
/// `InstanceEvent::topic()` for future subscription filtering.
fn emit_acp_event(app: &tauri::AppHandle, evt: crate::adapters::InstanceEvent) {
    use crate::adapters::InstanceEvent as GenEvt;
    let name = match &evt {
        GenEvt::State { .. } => "acp:instance-state",
        GenEvt::Transcript { .. } => "acp:transcript",
        GenEvt::PermissionRequest { .. } => "acp:permission-request",
        GenEvt::TurnStarted { .. } => "acp:turn-started",
        GenEvt::TurnEnded { .. } => "acp:turn-ended",
        GenEvt::InstancesChanged { .. } => "acp:instances-changed",
        GenEvt::InstancesFocused { .. } => "acp:instances-focused",
        GenEvt::Terminal { .. } => "acp:terminal",
        GenEvt::DaemonReloaded { .. } => "daemon:reloaded",
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
        let adapter = AcpAdapter::new(Config::default(), Arc::new(StatusBroadcast::new(true)));
        let err = adapter
            .submit_prompt("hi", &[], None, None, None)
            .await
            .expect_err("must fail");
        assert_eq!(err.code, -32602);
    }

    #[tokio::test]
    async fn info_empty_when_nothing_spawned() {
        let adapter = AcpAdapter::new(Config::default(), Arc::new(StatusBroadcast::new(true)));
        let v = adapter.info_json().await.expect("ok");
        assert_eq!(v["instances"], json!([]));
    }

    #[tokio::test]
    async fn cancel_unknown_agent_reports_missing_session() {
        let adapter = AcpAdapter::new(Config::default(), Arc::new(StatusBroadcast::new(true)));
        let err = adapter.cancel_active(None, Some("ghost")).await.expect_err("must fail");
        assert_eq!(err.code, -32602, "unknown agent id is invalid_params");
    }

    #[tokio::test]
    async fn cancel_invalid_instance_id_is_invalid_params() {
        let adapter = AcpAdapter::new(Config::default(), Arc::new(StatusBroadcast::new(true)));
        let err = adapter
            .cancel_active(Some("not-a-uuid"), None)
            .await
            .expect_err("must fail");
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
    async fn instance_key_rejects_empty_string() {
        let err = InstanceKey::parse("").expect_err("empty");
        assert!(matches!(err, AdapterError::InvalidRequest(_)));
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
        let adapter = AcpAdapter::new(cfg, Arc::new(StatusBroadcast::new(true)));

        let resolved = adapter.resolve(None, Some("strict")).expect("strict resolves");
        assert_eq!(resolved.agent.id, "claude-code");
        assert_eq!(resolved.profile_id.as_deref(), Some("strict"));
        assert_eq!(resolved.model.as_deref(), Some("claude-opus-4-5"));
        assert_eq!(resolved.system_prompt.as_deref(), Some("be terse"));

        let resolved = adapter.resolve(None, None).expect("default profile resolves");
        assert_eq!(resolved.profile_id.as_deref(), Some("ask"));
        assert_eq!(resolved.model.as_deref(), Some("claude-sonnet-4-5"));
        assert!(resolved.system_prompt.is_none());
    }

    #[tokio::test]
    async fn focus_nonexistent_is_invalid_params() {
        let adapter = AcpAdapter::new(Config::default(), Arc::new(StatusBroadcast::new(true)));
        let key = InstanceKey::parse("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let err = adapter.focus_instance(key).await.expect_err("unknown id must fail");
        assert_eq!(err.code, -32602);
    }

    #[tokio::test]
    async fn restart_nonexistent_is_invalid_params() {
        let adapter = AcpAdapter::new(Config::default(), Arc::new(StatusBroadcast::new(true)));
        let key = InstanceKey::parse("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let err = adapter.restart_instance(key).await.expect_err("unknown id must fail");
        assert_eq!(err.code, -32602);
    }

    #[tokio::test]
    async fn shutdown_one_nonexistent_is_invalid_params() {
        let adapter = AcpAdapter::new(Config::default(), Arc::new(StatusBroadcast::new(true)));
        let key = InstanceKey::parse("550e8400-e29b-41d4-a716-446655440000").unwrap();
        let err = adapter.shutdown_instance(key).await.expect_err("unknown id must fail");
        assert_eq!(err.code, -32602);
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
        let adapter = AcpAdapter::new(cfg, Arc::new(StatusBroadcast::new(true)));
        let out = adapter.list_profiles();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0]["id"], "ask");
        assert_eq!(out[0]["agent"], "claude-code");
        assert_eq!(out[0]["isDefault"], true);
        assert!(out[0].get("has_prompt").is_none());
        assert_eq!(out[1]["id"], "strict");
        assert_eq!(out[1]["isDefault"], false);
        assert!(out[1].get("has_prompt").is_none());
    }

    #[tokio::test]
    async fn list_commands_unknown_instance_id_is_invalid_params() {
        let adapter = AcpAdapter::new(Config::default(), Arc::new(StatusBroadcast::new(true)));
        let err = adapter
            .list_commands("550e8400-e29b-41d4-a716-446655440000")
            .await
            .expect_err("unknown id must fail");
        assert_eq!(err.code, -32602);
    }

    /// Mode threading: `spawn(SpawnSpec { mode: Some("plan"), ... })`
    /// lands on `InstanceInfo.mode` via the `AcpInstance` carry. Uses
    /// a config with a dead-child agent (so the spawn actor hits
    /// `Error` immediately) — the mode carry happens before the
    /// actor even starts, so the field is populated regardless.
    #[tokio::test]
    async fn spawn_threads_mode_through_to_instance_info() {
        let cfg: Config = toml::from_str(
            r#"
[agent]
default = "dead"

[[agents]]
id = "dead"
provider = "acp-claude-code"
command = "/bin/false"
"#,
        )
        .expect("parses");
        let adapter = AcpAdapter::new(cfg, Arc::new(StatusBroadcast::new(true)));
        let spec = SpawnSpec {
            mode: Some("plan".into()),
            ..Default::default()
        };
        let key = adapter.spawn_instance(spec).await.expect("spawn ok");
        let info = <AcpAdapter as Adapter>::info_for(&adapter, key)
            .await
            .expect("info_for");
        assert_eq!(info.mode.as_deref(), Some("plan"));
    }
}
