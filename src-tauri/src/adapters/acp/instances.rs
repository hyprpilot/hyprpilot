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

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use agent_client_protocol::schema::ListSessionsResponse;
use async_trait::async_trait;
use serde_json::{json, Value};
use tauri::Emitter;
use tokio::sync::{broadcast, mpsc, oneshot};

use super::instance::{AcpInstance, InstanceCommand};
use crate::adapters::instance::InstanceActor;
use crate::adapters::permission::{DefaultPermissionController, PermissionController};
use crate::adapters::profile::ResolvedInstance;
use crate::adapters::registry::AdapterRegistry;
use crate::adapters::{
    Adapter, AdapterError, AdapterId, AdapterResult, Bootstrap, InstanceEvent, InstanceEventStream, InstanceInfo,
    InstanceKey, InstanceState, SpawnSpec, UserTurnInput,
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
    /// Held only so the field appears in `Debug`. Future per-adapter
    /// status broadcasts will read it; rustc's dead-code lint
    /// doesn't count derived `Debug` impls as a use.
    #[allow(dead_code)]
    pub(crate) status: Arc<StatusBroadcast>,
    registry: Arc<AdapterRegistry<AcpInstance>>,
    permissions: Arc<dyn PermissionController>,
    /// Instance ids with at least one in-flight turn. Driven by a
    /// background task subscribed to the registry's broadcast —
    /// `TurnStarted` adds, `TurnEnded` removes. Read by
    /// `daemon/shutdown`'s busy check.
    busy_instances: Arc<RwLock<HashSet<String>>>,
    /// Slash-commands cache shared with the composer-autocomplete
    /// `CommandsSource`. Daemon installs it once at boot via
    /// [`Self::set_commands_cache`]; per-instance runtimes write to
    /// it on every `available_commands_update` notification.
    commands_cache: Arc<RwLock<Option<crate::completion::source::commands::CommandsCache>>>,
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
    /// Test-only convenience: builds a fresh shared config + default
    /// permissions controller. Production wiring goes through
    /// `with_shared_config` so `RpcState.config` and the adapter point
    /// at the same `Arc<RwLock<Config>>`. Narrow allow keeps this
    /// available to test sites without spamming dead-code warnings.
    #[allow(dead_code)]
    #[must_use]
    pub fn new(config: Config, status: Arc<StatusBroadcast>) -> Self {
        Self::with_permissions(
            config,
            status,
            Arc::new(DefaultPermissionController::new()) as Arc<dyn PermissionController>,
        )
    }

    #[allow(dead_code)]
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
            busy_instances: Arc::new(RwLock::new(HashSet::new())),
            commands_cache: Arc::new(RwLock::new(None)),
        }
    }

    /// Install the slash-commands cache. Called once at boot from
    /// `daemon::run` after the completion registry is built. Subsequent
    /// per-instance `available_commands_update` notifications populate
    /// it directly; the autocomplete `CommandsSource` reads from the
    /// same `Arc<RwLock<Vec<_>>>`.
    pub fn set_commands_cache(&self, cache: crate::completion::source::commands::CommandsCache) {
        *self.commands_cache.write().expect("commands_cache lock poisoned") = Some(cache);
    }

    /// Snapshot the configured commands cache handle (cheap Arc clone).
    /// Returns `None` when the cache hasn't been wired yet (early-boot
    /// race or test harness without completion registry).
    pub(crate) fn commands_cache(&self) -> Option<crate::completion::source::commands::CommandsCache> {
        self.commands_cache
            .read()
            .expect("commands_cache lock poisoned")
            .clone()
    }

    /// Resolved MCP file list for an instance. Profile's `mcps` when
    /// set wholesale-replaces the global `mcps`; None (unset) falls
    /// back. Used by the `mcps_list` Tauri command's readonly preview
    /// path AND by the ACP injection at session/new.
    pub(crate) fn effective_mcp_files_for(
        &self,
        profile: Option<&crate::config::ProfileConfig>,
    ) -> Vec<crate::config::ResolvedMcpFile> {
        if let Some(p) = profile {
            if let Some(files) = &p.mcps {
                return files
                    .iter()
                    .map(|e| crate::config::ResolvedMcpFile {
                        file: crate::paths::resolve_user(&e.file.to_string_lossy()),
                        ignore: e.compile_ignore(),
                    })
                    .collect();
            }
        }
        self.read_config().resolved_mcps()
    }

    /// Per-instance MCP catalog as a flat `Vec<MCPDefinition>`. Drives
    /// the `mcps_list` Tauri command's preview pane: when `instance_id`
    /// resolves to a live actor we use that instance's profile to pick
    /// the right MCP files, otherwise we fall back to the global set.
    /// Without this lookup the captain's profile-scoped MCPs are
    /// invisible in the palette while still being injected into ACP at
    /// session/new — a silent divergence between what the agent sees
    /// and what the UI shows.
    pub async fn resolve_mcp_catalog(&self, instance_id: Option<&str>) -> Vec<crate::mcp::MCPDefinition> {
        let profile = match instance_id.and_then(|id| InstanceKey::parse(id).ok()) {
            Some(key) => match self.registry.info_for(key).await {
                Ok(info) => self.profile_by_id(info.profile_id.as_deref()),
                Err(_) => None,
            },
            None => None,
        };
        let files = self.effective_mcp_files_for(profile.as_ref());
        crate::mcp::loader::load_files(&files)
    }

    /// Build a per-instance `MCPsRegistry` from the resolved file list.
    /// Returns `None` when no files are configured (so the permission
    /// pipeline's lane 2 stays inactive and the call site doesn't pay
    /// for an empty-registry deref). Per-file load errors warn + skip
    /// inside `loader::load_files`.
    fn build_mcp_registry_for(
        &self,
        profile: Option<&crate::config::ProfileConfig>,
    ) -> Option<Arc<crate::mcp::MCPsRegistry>> {
        let files = self.effective_mcp_files_for(profile);
        if files.is_empty() {
            return None;
        }
        let defs = crate::mcp::loader::load_files(&files);
        if defs.is_empty() {
            return None;
        }
        Some(Arc::new(crate::mcp::MCPsRegistry::new(defs)))
    }

    /// Resolved skill-root list for an instance. Profile's `skills`
    /// wholesale-replaces the global `[[skills]]`; `None` (unset)
    /// falls back. Mirror of `effective_mcp_files_for`. Drives the
    /// per-instance `SkillsRegistry` built once at spawn time.
    pub(crate) fn effective_skills_for(
        &self,
        profile: Option<&crate::config::ProfileConfig>,
    ) -> Vec<crate::config::ResolvedSkillEntry> {
        if let Some(p) = profile {
            if let Some(entries) = &p.skills {
                return entries
                    .iter()
                    .map(|e| crate::config::ResolvedSkillEntry {
                        dir: crate::paths::resolve_user(&e.dir.to_string_lossy()),
                        ignore: e.compile_ignore(),
                    })
                    .collect();
            }
        }
        self.read_config().resolved_skills()
    }

    /// Build the per-instance `SkillsRegistry` from the resolved
    /// entries. Calls `reload()` immediately so the registry is
    /// ready-to-list at spawn time (no first-prompt lag while disk
    /// walks). Empty registries are valid — captain may have
    /// `skills = []` set explicitly. `reload()` errors are logged
    /// and swallowed; the captain can hit `skills/reload` to retry.
    fn build_skills_registry_for(
        &self,
        profile: Option<&crate::config::ProfileConfig>,
    ) -> Arc<crate::skills::SkillsRegistry> {
        let entries = self.effective_skills_for(profile);
        let registry = Arc::new(crate::skills::SkillsRegistry::new(entries));
        if let Err(err) = registry.reload() {
            tracing::warn!(%err, "acp::adapter: per-instance skills initial reload failed");
        }
        registry
    }

    /// Per-instance skills registry for an addressed key. Returns
    /// `None` when the key isn't live. The registry is the per-spawn
    /// view filtered by the instance's profile — the only source of
    /// truth for the palette / autocomplete / hydrator.
    pub async fn instance_skills(&self, key: InstanceKey) -> Option<Arc<crate::skills::SkillsRegistry>> {
        self.registry.get(key).await.map(|h| h.skills.clone())
    }

    /// Per-instance skills registry for the focused instance. `None`
    /// when no instance is focused (boot pre-spawn / all-shutdown
    /// states). Drives the composer autocomplete + inline-token
    /// hydrator — both ride the focused instance's filter.
    pub async fn focused_skills(&self) -> Option<Arc<crate::skills::SkillsRegistry>> {
        let key = self.registry.focused_id().await?;
        self.instance_skills(key).await
    }

    /// Reload every live instance's skills registry from disk.
    /// Returns the aggregate skill count across all instances post-
    /// reload — the figure the `daemon/reload` RPC surfaces in its
    /// response. Per-instance reload errors are logged and skipped;
    /// aggregate stays valid (the broken instance's count drops to
    /// 0, the rest add normally).
    pub async fn reload_all_skills(&self) -> usize {
        let mut total = 0usize;
        for key in self.registry.ordered_keys().await {
            let Some(handle) = self.registry.get(key).await else {
                continue;
            };
            if let Err(err) = handle.skills.reload() {
                tracing::warn!(
                    instance = %key,
                    %err,
                    "acp::adapter: per-instance skills reload failed",
                );
                continue;
            }
            total += handle.skills.list().len();
        }
        total
    }

    /// Handle onto the shared config. Used by the daemon wiring to
    /// hand the same lock to `RpcState` so reads + writes stay
    /// coherent. Test-only consumer today (real daemon constructs the
    /// adapter via `with_shared_config` then passes the same `Arc`
    /// straight into `RpcState`); narrow allow keeps the accessor.
    #[allow(dead_code)]
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

    /// Snapshot of every instance id currently mid-turn. Used by
    /// `daemon/shutdown` for the busy check.
    pub fn busy_instance_ids(&self) -> impl std::future::Future<Output = Vec<String>> + Send {
        let busy = self.busy_instances.clone();
        async move { busy.read().map(|set| set.iter().cloned().collect()).unwrap_or_default() }
    }

    /// Publish a `DaemonReloaded` event onto the registry's broadcast.
    /// Will be invoked by the `daemon/reload` RPC handler (OP1) after
    /// the config + skills rescans complete. Narrow allow until that
    /// handler arm lands in this same MR.
    #[allow(dead_code)]
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

    /// Test hook: insert a stub `AcpInstance` whose `mirror` Arc is
    /// addressable via [`Adapter::instance_mirror`]. Bypasses the
    /// actor spawn — `cmd_tx` is bound to a dropped receiver, so
    /// commands fail closed (good for snapshot RPC tests, which
    /// only read the mirror). Returns the inserted key so tests can
    /// thread it into wire-shaped params.
    #[cfg(test)]
    pub async fn test_install_mirror(
        &self,
        mirror: std::sync::Arc<crate::adapters::mirror::InstanceMirror>,
    ) -> InstanceKey {
        let key = InstanceKey::new_v4();
        let handle = std::sync::Arc::new(crate::adapters::acp::instance::AcpInstance::stub_for_tests(key, mirror));
        self.registry.insert(key, handle, None).await.expect("test insert");
        key
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
    #[allow(dead_code)]
    #[must_use]
    pub fn subscribe_events(&self) -> broadcast::Receiver<crate::adapters::InstanceEvent> {
        self.registry.subscribe()
    }

    /// Sender side of the same broadcast `subscribe_events` reads
    /// from. Used by the daemon to wire the broadcast back into the
    /// permission controller post-construction so
    /// `PermissionResolved` events surface alongside the rest of the
    /// instance event stream.
    #[must_use]
    pub fn events_tx(&self) -> broadcast::Sender<crate::adapters::InstanceEvent> {
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

        // Busy tracker — subscribes to the same registry broadcast and
        // maintains `busy_instances` off `TurnStarted` / `TurnEnded`.
        // Co-located with the Tauri event bridge so the spawn lands in
        // the Tauri runtime context (daemon `.setup(...)` closure).
        // Uses `tauri::async_runtime::spawn` because `setup` is a sync
        // closure — there's no current tokio reactor to call plain
        // `tokio::spawn` against.
        let mut busy_rx = self.registry.subscribe();
        let busy = self.busy_instances.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                match busy_rx.recv().await {
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
                    // Defensive cleanup on actor termination — covers
                    // crash paths that bypass the `TurnGuard` drop and
                    // leave a stale "busy" entry forever. Any non-live
                    // state means there's no actor to be busy on
                    // anyway.
                    Ok(InstanceEvent::State {
                        instance_id,
                        state: InstanceState::Ended | InstanceState::Error,
                        ..
                    }) => {
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

        // Status bridge — drives the `StatusBroadcast` snapshot off the
        // ACP turn lifecycle so waybar's `ctl status --watch` reflects
        // what the agent is actually doing. TurnStarted → Streaming;
        // PermissionRequest → Awaiting (only while a turn is open);
        // TurnEnded → Idle (or Error when the turn carried an error).
        let mut status_rx = self.registry.subscribe();
        let status = self.status.clone();
        tauri::async_runtime::spawn(async move {
            loop {
                match status_rx.recv().await {
                    Ok(InstanceEvent::TurnStarted { session_id, .. }) => {
                        status.set_state(crate::rpc::protocol::AgentState::Streaming, Some(session_id));
                    }
                    Ok(InstanceEvent::PermissionRequest { session_id, .. }) => {
                        status.set_state(crate::rpc::protocol::AgentState::Awaiting, Some(session_id));
                    }
                    Ok(InstanceEvent::TurnEnded { error, .. }) => {
                        let next = if error.is_some() {
                            crate::rpc::protocol::AgentState::Error
                        } else {
                            crate::rpc::protocol::AgentState::Idle
                        };
                        status.set_state(next, None);
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(n, "acp status bridge: lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => return,
                }
            }
        });
    }

    /// Resolve a `(agent_id?, profile_id?)` pair. When both are
    /// omitted, falls back through `[profile] default` and finally
    /// to `[agent] default`. Explicit `agent_id` overrides whatever
    /// agent the resolved profile names (same profile, new agent
    /// spawn).
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
        // Per-instance MCP catalog: profile's `mcps` wholesale-
        // replaces the global default; the resolved set is what
        // `PermissionController::decide` lane 2 reads via
        // `DecisionContext.mcps`. None when no MCP files are wired —
        // the per-server lane short-circuits and every call falls
        // through to AskUser (or trust store).
        let mcps = self.build_mcp_registry_for(profile.as_ref());
        let skills = self.build_skills_registry_for(profile.as_ref());
        let instance = AcpInstance::start(crate::adapters::acp::instance::StartParams {
            resolved,
            key,
            profile_id,
            events_tx: self.registry.events_tx(),
            bootstrap,
            permissions: self.permissions.clone(),
            mcps,
            skills,
            commands_cache: self.commands_cache(),
        });

        self.registry
            .insert(key, Arc::new(instance), None)
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
            has_prompt = !resolved.system_prompt.is_empty(),
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
    /// programmatic consumers. Test-only consumer today.
    #[allow(dead_code)]
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
            // Ephemeral list-only actors must NOT publish onto the
            // registry's UI-bound broadcast — their state transitions
            // (Starting → Running → Ended) leak as if a real session
            // came and went. The UI's `useSessionHistory` listens for
            // `Ended` to refresh, and refresh re-spawns an ephemeral
            // actor — instant infinite loop. Route their events into
            // a private sink channel that the daemon owns + drops
            // after the list response resolves.
            // `_unread_rx` keeps the broadcast channel open for the
            // sender — drop it after the list resolves and the actor
            // shuts itself down.
            let (sink_tx, _unread_rx) = broadcast::channel::<crate::adapters::InstanceEvent>(8);
            // Ephemeral list-only actor never reads MCPs / skills;
            // pass empty registries so the actor body's accessors
            // stay non-Option without paying for a disk walk on the
            // throwaway path.
            let _ = profile;
            let instance = AcpInstance::start(crate::adapters::acp::instance::StartParams {
                resolved,
                key: ephemeral_key,
                profile_id: profile_id_for_instance,
                events_tx: sink_tx,
                bootstrap: Bootstrap::ListOnly,
                permissions: self.permissions.clone(),
                mcps: None,
                skills: Arc::new(crate::skills::SkillsRegistry::new(Vec::new())),
                commands_cache: None,
            });
            let tx = instance.cmd_tx.clone();
            (tx, Some(instance))
        };

        let (reply_tx, reply_rx) = oneshot::channel();
        cmd_tx
            .send(InstanceCommand::ListSessions { cwd, reply: reply_tx })
            .map_err(|_| RpcError::internal_error("instance actor closed before accepting list request"))?;

        let mut response = reply_rx
            .await
            .map_err(|_| RpcError::internal_error("instance actor dropped list reply"))?
            .map_err(|err| RpcError::internal_error(format!("session/list failed: {err}")));

        if let Some(handle) = ephemeral {
            handle.shutdown().await;
        }

        // Display-format every session's cwd so the wire shape matches
        // `InstanceMeta { cwd }` byte-for-byte. Without this the UI's
        // `s.cwd === activeInstance.cwd` filter compares the agent-
        // persisted absolute against our display-formatted (`~/...`)
        // active cwd and yields the spurious "no sessions" the
        // captain hit. Mutating `cwd` on the non-exhaustive struct is
        // allowed since fields stay public.
        if let Ok(ref mut r) = response {
            for s in r.sessions.iter_mut() {
                let display = crate::tools::path::display_cwd(&s.cwd.to_string_lossy());
                s.cwd = display.into();
            }
        }

        response
    }

    /// Resume a persisted session. `instance_id` addresses the live
    /// (or new) instance to bind the loaded session into — when
    /// omitted, mints a fresh key. Tears down the live actor at that
    /// key if present, then spawns with `Bootstrap::Resume(session_id)`.
    /// Auto-focuses the resumed instance so the UI's transcript view +
    /// header chrome flip onto it without the caller threading focus
    /// separately. claude-agent-acp (and any spec-compliant ACP agent)
    /// streams `session/update` notifications WHILE servicing the
    /// `LoadSessionRequest` to replay prior turns; those notifications
    /// land on the resumed instance's slot and only become visible if
    /// the user is actually watching that instance.
    pub async fn load_session(
        &self,
        instance_id: Option<&str>,
        agent_id: Option<&str>,
        profile_id: Option<&str>,
        session_id: String,
        cwd: Option<PathBuf>,
    ) -> Result<InstanceKey, RpcError> {
        let key = match instance_id {
            Some(s) => InstanceKey::parse(s).map_err(map_adapter_error_to_rpc)?,
            None => InstanceKey::new_v4(),
        };
        let mut resolved = self.resolve(agent_id, profile_id)?;
        // Override the profile-default cwd with the session's own. ACP
        // agents (claude-agent-acp) scope persisted sessions BY cwd —
        // resuming session-X under any cwd other than the one it was
        // created with returns "Resource not found". The UI knows the
        // session's cwd from `session_list`; thread it through here so
        // the resume request lands in the right scope.
        if let Some(c) = cwd {
            resolved.agent.cwd = Some(c);
        }
        self.ensure(key, resolved, Bootstrap::Resume(session_id)).await?;
        self.registry.focus(key).await.map_err(map_adapter_error_to_rpc)?;
        Ok(key)
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
    /// behaves consistently. Optional `cwd` overlays
    /// `resolved.agent.cwd` before the new actor spawns so the cwd
    /// palette can swap working directories without a full
    /// shutdown / respawn cycle.
    ///
    /// When `ensure` is true and `key` doesn't resolve to a live
    /// handle (or is `None`), falls through to `spawn_instance` with
    /// `(profile_id, agent_id, cwd)` so the cwd palette gets a fresh
    /// instance rooted at the requested cwd on empty registry. The
    /// daemon-side ensure mirrors `instance_meta_or_ensure`.
    pub async fn restart_instance(
        &self,
        key: Option<InstanceKey>,
        cwd: Option<PathBuf>,
        ensure: bool,
        agent_id: Option<&str>,
        profile_id: Option<&str>,
    ) -> Result<InstanceKey, RpcError> {
        if let Some(c) = &cwd {
            if !c.is_dir() {
                return Err(RpcError::invalid_params(format!(
                    "cwd '{}' is not an existing directory",
                    c.display()
                )));
            }
        }

        let live = match key {
            Some(k) => self.registry.get(k).await.map(|h| (k, h)),
            None => None,
        };

        let (key, existing) = match live {
            Some(pair) => pair,
            None => {
                // No live handle — delegate to the shared resolve-or-
                // spawn helper. When `ensure`, spawns under `spec`
                // rooted at `cwd`; otherwise returns the not-found
                // error. We don't need the spawned handle here, just
                // its key.
                let spec = SpawnSpec {
                    profile_id: profile_id.map(str::to_string),
                    agent_id: agent_id.map(str::to_string),
                    cwd,
                    mode: None,
                    model: None,
                };
                let (new_key, _handle) = self.resolve_or_spawn(key, ensure, spec).await?;
                return Ok(new_key);
            }
        };

        let existing_agent_id = existing.agent_id.clone();
        let existing_profile_id = existing.profile_id.clone();
        let mode = existing.mode.clone();
        drop(existing);

        let slot = self
            .registry
            .drop_preserving_slot(key)
            .await
            .map_err(map_adapter_error_to_rpc)?;

        let mut resolved = self.resolve(Some(&existing_agent_id), existing_profile_id.as_deref())?;
        if mode.is_some() {
            resolved.mode = mode;
        }
        if let Some(c) = cwd {
            resolved.agent.cwd = Some(c);
        }
        let profile = self.profile_by_id(resolved.profile_id.as_deref());
        let profile_id_for_instance = resolved.profile_id.clone();
        let mcps = self.build_mcp_registry_for(profile.as_ref());
        let skills = self.build_skills_registry_for(profile.as_ref());
        let instance = AcpInstance::start(crate::adapters::acp::instance::StartParams {
            resolved,
            key,
            profile_id: profile_id_for_instance,
            events_tx: self.registry.events_tx(),
            bootstrap: Bootstrap::Fresh,
            permissions: self.permissions.clone(),
            mcps,
            skills,
            commands_cache: self.commands_cache(),
        });
        self.registry
            .insert(key, Arc::new(instance), Some(slot))
            .await
            .map_err(map_adapter_error_to_rpc)?;
        Ok(key)
    }

    /// Enumerate configured agents for `agents_list`. Typed wire
    /// shape — `skip_serializing_if` on optional fields keeps null
    /// off the wire, matching the no-fabrication invariant the
    /// `InstanceListEntry` migration nailed down.
    #[must_use]
    pub fn list_agents(&self) -> Vec<crate::adapters::AgentSummary> {
        let cfg = self.read_config();
        let default_agent = cfg.agents.agent.default.as_deref();
        cfg.agents
            .agents
            .iter()
            .map(|a| crate::adapters::AgentSummary {
                id: a.id.clone(),
                provider: format!("{:?}", a.provider).to_ascii_lowercase().replace('_', "-"),
                binding: a.command.clone(),
                is_default: default_agent == Some(a.id.as_str()),
            })
            .collect()
    }

    /// Enumerate configured profiles for `config/profiles` +
    /// `profiles/list`.
    pub fn list_profiles(&self) -> Vec<crate::adapters::ProfileSummary> {
        let cfg = self.read_config();
        let default_profile = cfg.profile.default.as_deref();
        cfg.profiles
            .iter()
            .map(|p| crate::adapters::ProfileSummary {
                id: p.id.clone(),
                agent: p.agent.clone(),
                model: p.model.clone(),
                is_default: default_profile == Some(p.id.as_str()),
            })
            .collect()
    }

    /// Shutdown a single instance and auto-focus the oldest survivor.
    /// Test-only consumer today; production handlers reach for
    /// `Adapter::shutdown_one` via the trait.
    #[allow(dead_code)]
    pub async fn shutdown_instance(&self, key: InstanceKey) -> Result<InstanceKey, RpcError> {
        let key = self
            .registry
            .shutdown_one(key)
            .await
            .map_err(map_adapter_error_to_rpc)?;
        Ok(key)
    }

    /// Designate the focused instance. Unknown id → `-32602 invalid_params`.
    /// Test-only consumer today; production reaches `Adapter::focus`.
    #[allow(dead_code)]
    pub async fn focus_instance(&self, key: InstanceKey) -> Result<InstanceKey, RpcError> {
        self.registry.focus(key).await.map_err(map_adapter_error_to_rpc)
    }

    /// Resolve `instance_id` to a live `Arc<AcpInstance>` in one step.
    /// The single `registry.get` here closes the TOCTOU window that a
    /// parse + membership-check + handle-clone pattern would open:
    /// the registry's RwLock doesn't release between parse and lookup,
    /// so the instance can't be shut down mid-resolve. Used by the
    /// `set_*` handlers that require an existing live actor.
    async fn require_instance(&self, instance_id: &str) -> Result<Arc<AcpInstance>, RpcError> {
        let key = InstanceKey::parse(instance_id).map_err(map_adapter_error_to_rpc)?;
        self.registry
            .get(key)
            .await
            .ok_or_else(|| RpcError::invalid_params(format!("instance '{instance_id}' not found in registry")))
    }

    /// Switch the active model on the addressed instance. Routes to
    /// the per-instance actor's `SetModel` command, which sends ACP
    /// `session/set_model` (gated by `unstable_session_model`).
    pub async fn set_session_model(&self, instance_id: &str, model_id: &str) -> Result<Value, RpcError> {
        let handle = self.require_instance(instance_id).await?;
        handle
            .set_model(model_id.to_string())
            .await
            .map_err(RpcError::internal_error)?;
        Ok(serde_json::json!({ "modelId": model_id }))
    }

    /// Switch the active mode on the addressed instance. Routes to
    /// the per-instance actor's `SetMode` command, which sends ACP
    /// `session/set_mode`.
    pub async fn set_session_mode(&self, instance_id: &str, mode_id: &str) -> Result<Value, RpcError> {
        let handle = self.require_instance(instance_id).await?;
        handle
            .set_mode(mode_id.to_string())
            .await
            .map_err(RpcError::internal_error)?;
        Ok(serde_json::json!({ "modeId": mode_id }))
    }

    /// Set a generic ACP `session/set_config_option`. The agent
    /// advertises the available config options on
    /// `NewSessionResponse.configOptions`; the captain picks one of
    /// the offered values for a given config_id, and the agent
    /// responds with the full updated `configOptions` array (which it
    /// also typically pushes via a `config_option_update`
    /// notification). This catch-all surface covers spec-reserved
    /// categories — `mode` / `model` / `thought_level` — when the
    /// agent surfaces them on configOptions instead of the dedicated
    /// `set_mode` / `set_model` methods, AND every vendor-specific
    /// `_*` category (per spec, `_*` ids are free for custom use).
    ///
    /// Usage example — claude-code surfaces a `thought_level` selector:
    /// ```ignore
    /// // session/new response carried:
    /// // configOptions: [{ id: "thought_level", currentValue: "low",
    /// //                    options: { type: "select", options: [
    /// //                      { id: "low", name: "Low" }, … ] } }]
    /// adapter.set_session_config_option(iid, "thought_level", "high").await
    /// ```
    pub async fn set_session_config_option(
        &self,
        instance_id: &str,
        config_id: &str,
        value: &str,
    ) -> Result<Value, RpcError> {
        let handle = self.require_instance(instance_id).await?;
        handle
            .set_config_option(config_id.to_string(), value.to_string())
            .await
            .map_err(RpcError::internal_error)?;
        Ok(serde_json::json!({ "configId": config_id, "value": value }))
    }

    /// Read the addressed instance's per-instance metadata cache.
    /// The palette pickers (modes, models) call this on every open
    /// so the listed options come straight from the daemon's
    /// authoritative cache instead of a UI-side mirror that may lag
    /// the latest `acp:instance-meta` event.
    pub async fn instance_meta(&self, instance_id: &str) -> Result<Value, RpcError> {
        let handle = self.require_instance(instance_id).await?;
        let snap = handle.meta_snapshot().await.map_err(RpcError::internal_error)?;
        serde_json::to_value(snap).map_err(|e| RpcError::internal_error(e.to_string()))
    }

    /// Resolve `key` to a live `(key, handle)` pair, or spawn a fresh
    /// actor under `spec` when `ensure` is true. The freshly-spawned
    /// actor adopts `key` if given (so caller-supplied UUIDs survive
    /// across the spawn — important for the webview's
    /// "push-to-store-then-RPC" pattern); otherwise mints a new UUID.
    /// On miss + !ensure, returns the standard not-found error.
    ///
    /// Shared by every palette flow that needs to act on an instance
    /// regardless of whether one was live: meta snapshot for the
    /// picker leaves, cwd restart on empty registry, future prewarm
    /// callers.
    async fn resolve_or_spawn(
        &self,
        key: Option<InstanceKey>,
        ensure: bool,
        spec: SpawnSpec,
    ) -> Result<(InstanceKey, Arc<AcpInstance>), RpcError> {
        if let Some(k) = key {
            if let Some(handle) = self.registry.get(k).await {
                return Ok((k, handle));
            }
        }
        if !ensure {
            let key_str = key.map_or_else(|| "<none>".to_string(), |k| k.to_string());
            return Err(RpcError::invalid_params(format!(
                "instance '{key_str}' not found in registry"
            )));
        }
        let mut resolved = self.resolve(spec.agent_id.as_deref(), spec.profile_id.as_deref())?;
        if let Some(c) = spec.cwd {
            resolved.agent.cwd = Some(c);
        }
        if spec.model.is_some() {
            resolved.model = spec.model;
        }
        if spec.mode.is_some() {
            resolved.mode = spec.mode;
        }
        let new_key = key.unwrap_or_else(InstanceKey::new_v4);
        let new_key = self.ensure(new_key, resolved, Bootstrap::Fresh).await?;
        let handle = self
            .registry
            .get(new_key)
            .await
            .ok_or_else(|| RpcError::internal_error("instance actor vanished after ensure"))?;
        Ok((new_key, handle))
    }

    /// Same as `instance_meta`, but transparently spawns an instance
    /// from `(agent_id, profile_id)` when `instance_id` is absent or
    /// doesn't resolve to a live actor. Drives the palette's
    /// "no active instance — let me bootstrap one for you" path:
    /// captain hits Ctrl+K → models on a clean overlay, daemon
    /// spawns the resolved profile in-place, returns the freshly-
    /// loaded meta with available models populated.
    ///
    /// `MetaSnapshot` waits for the actor's command loop, which only
    /// starts processing after `session/new` completes — so the
    /// returned snapshot already has `availableModels` /
    /// `availableModes` populated by the agent's initialize handshake.
    pub async fn instance_meta_or_ensure(
        &self,
        instance_id: Option<&str>,
        agent_id: Option<&str>,
        profile_id: Option<&str>,
    ) -> Result<Value, RpcError> {
        let key = match instance_id {
            Some(s) => Some(InstanceKey::parse(s).map_err(map_adapter_error_to_rpc)?),
            None => None,
        };
        let spec = SpawnSpec {
            profile_id: profile_id.map(str::to_string),
            agent_id: agent_id.map(str::to_string),
            cwd: None,
            mode: None,
            model: None,
        };
        let (key, handle) = self.resolve_or_spawn(key, true, spec).await?;
        let snap = handle.meta_snapshot().await.map_err(RpcError::internal_error)?;
        augment_with_instance_id(snap, &key)
    }
}

/// Project a `MetaSnapshot` to JSON and graft the resolved
/// `instanceId` alongside its fields. The ensure flow returns the
/// id of the (possibly freshly-spawned) actor so the caller can
/// route follow-up commands without round-tripping useActiveInstance
/// (which updates async via the registry's auto-focus event).
fn augment_with_instance_id(
    snap: crate::adapters::acp::instance::MetaSnapshot,
    key: &InstanceKey,
) -> Result<Value, RpcError> {
    let mut value = serde_json::to_value(snap).map_err(|e| RpcError::internal_error(e.to_string()))?;
    if let Value::Object(map) = &mut value {
        map.insert("instanceId".to_string(), Value::String(key.as_string()));
    }
    Ok(value)
}

#[async_trait]
impl Adapter for AcpAdapter {
    fn id(&self) -> AdapterId {
        AdapterId::Acp
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

    async fn restart(&self, key: InstanceKey, cwd: Option<PathBuf>) -> AdapterResult<InstanceKey> {
        self.restart_instance(Some(key), cwd, false, None, None)
            .await
            .map_err(rpc_to_adapter)
    }

    async fn resolve_token(&self, token: &str) -> Option<InstanceKey> {
        self.registry.resolve_token(token).await
    }

    async fn rename(&self, key: InstanceKey, name: Option<String>) -> AdapterResult<()> {
        self.registry.rename(key, name).await
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

    fn permissions(&self) -> Option<std::sync::Arc<dyn crate::adapters::permission::PermissionController>> {
        Some(AcpAdapter::permissions(self))
    }

    async fn instance_mirror(
        &self,
        key: InstanceKey,
    ) -> Option<std::sync::Arc<crate::adapters::mirror::InstanceMirror>> {
        self.registry.get(key).await.map(|h| h.mirror.clone())
    }

    // ── wire-method dispatch (S3 expansion) ───────────────────────────

    async fn list_agents(&self) -> AdapterResult<Vec<crate::adapters::AgentSummary>> {
        Ok(AcpAdapter::list_agents(self))
    }

    async fn list_profiles(&self) -> AdapterResult<Vec<crate::adapters::ProfileSummary>> {
        Ok(AcpAdapter::list_profiles(self))
    }

    async fn set_session_model(&self, instance_id: &str, model_id: &str) -> AdapterResult<serde_json::Value> {
        AcpAdapter::set_session_model(self, instance_id, model_id)
            .await
            .map_err(rpc_to_adapter)
    }

    async fn set_session_mode(&self, instance_id: &str, mode_id: &str) -> AdapterResult<serde_json::Value> {
        AcpAdapter::set_session_mode(self, instance_id, mode_id)
            .await
            .map_err(rpc_to_adapter)
    }

    async fn set_session_config_option(
        &self,
        instance_id: &str,
        config_id: &str,
        value: &str,
    ) -> AdapterResult<serde_json::Value> {
        AcpAdapter::set_session_config_option(self, instance_id, config_id, value)
            .await
            .map_err(rpc_to_adapter)
    }

    async fn list_sessions(
        &self,
        instance_id: Option<&str>,
        agent_id: Option<&str>,
        profile_id: Option<&str>,
        cwd: Option<PathBuf>,
    ) -> AdapterResult<Value> {
        let resp = AcpAdapter::list_sessions(self, instance_id, agent_id, profile_id, cwd)
            .await
            .map_err(rpc_to_adapter)?;
        serde_json::to_value(resp)
            .map_err(|err| AdapterError::Backend(format!("serialize ListSessionsResponse: {err}")))
    }

    async fn load_session(
        &self,
        instance_id: Option<&str>,
        agent_id: Option<&str>,
        profile_id: Option<&str>,
        session_id: String,
        cwd: Option<PathBuf>,
    ) -> AdapterResult<InstanceKey> {
        AcpAdapter::load_session(self, instance_id, agent_id, profile_id, session_id, cwd)
            .await
            .map_err(rpc_to_adapter)
    }

    async fn busy_instance_ids(&self) -> Vec<String> {
        AcpAdapter::busy_instance_ids(self).await
    }

    fn publish_daemon_reloaded(&self, profiles: usize, skills_count: usize, mcps_count: usize) {
        AcpAdapter::publish_daemon_reloaded(self, profiles, skills_count, mcps_count);
    }

    async fn reload_all_skills(&self) -> usize {
        AcpAdapter::reload_all_skills(self).await
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
        GenEvt::PermissionResolved { .. } => "acp:permission-resolved",
        GenEvt::TurnStarted { .. } => "acp:turn-started",
        GenEvt::TurnEnded { .. } => "acp:turn-ended",
        GenEvt::InstancesChanged { .. } => "acp:instances-changed",
        GenEvt::InstancesFocused { .. } => "acp:instances-focused",
        GenEvt::InstanceRenamed { .. } => "acp:instance-renamed",
        GenEvt::Terminal { .. } => "acp:terminal",
        GenEvt::DaemonReloaded { .. } => "daemon:reloaded",
        GenEvt::SessionInfoUpdate { .. } => "acp:session-info-update",
        GenEvt::CurrentModeUpdate { .. } => "acp:current-mode-update",
        GenEvt::UsageUpdate { .. } => "acp:usage-update",
        GenEvt::ConfigOptionsUpdate { .. } => "acp:config-options-update",
        GenEvt::InstanceMeta { .. } => "acp:instance-meta",
        GenEvt::SystemPromptInjected { .. } => "acp:system-prompt-injected",
    };
    match serde_json::to_value(&evt) {
        Ok(v) => {
            // Same split as `acp::emit` / `snapshot::mirror` — chunk
            // emits (transcript / terminal output) ride their own
            // sub-target so the lifecycle stream stays readable at
            // trace level. Opt into chunk emits via
            // `tauri::emit::chunk=trace`.
            if matches!(evt, GenEvt::Transcript { .. } | GenEvt::Terminal { .. }) {
                tracing::trace!(
                    target: "tauri::emit::chunk",
                    event = name,
                    topic = evt.topic(),
                    "emitting tauri event to webview (chunk)",
                );
            } else {
                tracing::trace!(
                    target: "tauri::emit",
                    event = name,
                    topic = evt.topic(),
                    "emitting tauri event to webview",
                );
            }
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
        let dir = tempfile::tempdir().unwrap();
        let prompt_path = dir.path().join("strict.md");

        std::fs::write(&prompt_path, "be terse").unwrap();

        let cfg: Config = toml::from_str(&format!(
            r#"
[agent]
default = "claude-code"

[profile]
default = "ask"

[[agents]]
id = "claude-code"
provider = "acp-claude-code"
command = "bunx"
model = "claude-sonnet-4-5"

[[profiles]]
id = "ask"
agent = "claude-code"

[[profiles]]
id = "strict"
agent = "claude-code"
model = "claude-opus-4-5"
system_prompt = [{{ file = "{}" }}]
"#,
            prompt_path.display()
        ))
        .expect("fixture parses");
        let adapter = AcpAdapter::new(cfg, Arc::new(StatusBroadcast::new(true)));

        let resolved = adapter.resolve(None, Some("strict")).expect("strict resolves");
        assert_eq!(resolved.agent.id, "claude-code");
        assert_eq!(resolved.profile_id.as_deref(), Some("strict"));
        assert_eq!(resolved.model.as_deref(), Some("claude-opus-4-5"));
        assert_eq!(
            resolved.system_prompt_for(&Bootstrap::Fresh).as_deref(),
            Some("be terse")
        );

        let resolved = adapter.resolve(None, None).expect("default profile resolves");
        assert_eq!(resolved.profile_id.as_deref(), Some("ask"));
        assert_eq!(resolved.model.as_deref(), Some("claude-sonnet-4-5"));
        assert!(resolved.system_prompt_for(&Bootstrap::Fresh).is_none());
    }

    fn skills_fixture_config(skills_dir: &std::path::Path) -> Config {
        // Three profiles all pointing at the same skill dir with
        // DIFFERENT ignore globs — the canonical "stale daemon-global"
        // scenario the per-instance refactor exists to fix. Each
        // profile's spawned instance must see its own filter, never
        // the first-iterated profile's view leaking across.
        toml::from_str(&format!(
            r#"
[agent]
default = "claude-code"

[profile]
default = "personal"

[[agents]]
id = "claude-code"
provider = "acp-claude-code"
command = "/bin/false"

[[profiles]]
id = "personal"
agent = "claude-code"
skills = [{{ dir = "{dir}", ignore = ["work-*"] }}]

[[profiles]]
id = "work"
agent = "claude-code"
skills = [{{ dir = "{dir}", ignore = ["personal-*"] }}]

[[profiles]]
id = "no-skills"
agent = "claude-code"
skills = []
"#,
            dir = skills_dir.display(),
        ))
        .expect("fixture parses")
    }

    fn seed_skill(root: &std::path::Path, slug: &str) {
        let dir = root.join(slug);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("SKILL.md"),
            format!("---\ndescription: {slug}\n---\n\n# {slug}\n\nbody\n"),
        )
        .unwrap();
    }

    /// Build a registry from the addressed profile's filter and
    /// confirm that profile's globs apply — i.e. NOT the global /
    /// first-profile's globs. This is the regression test for the
    /// "stale daemon-global" bug.
    #[test]
    fn build_skills_registry_for_uses_addressed_profile_globs() {
        let tmp = tempfile::tempdir().unwrap();
        seed_skill(tmp.path(), "personal-todo");
        seed_skill(tmp.path(), "work-internal");
        seed_skill(tmp.path(), "shared-readme");
        let cfg = skills_fixture_config(tmp.path());
        let adapter = AcpAdapter::new(cfg, Arc::new(StatusBroadcast::new(true)));

        let personal = adapter
            .read_config()
            .profiles
            .iter()
            .find(|p| p.id == "personal")
            .cloned();
        let work = adapter.read_config().profiles.iter().find(|p| p.id == "work").cloned();
        let no_skills = adapter
            .read_config()
            .profiles
            .iter()
            .find(|p| p.id == "no-skills")
            .cloned();

        let personal_reg = adapter.build_skills_registry_for(personal.as_ref());
        let personal_slugs: Vec<String> = personal_reg.list().iter().map(|s| s.slug.to_string()).collect();
        assert!(personal_slugs.contains(&"personal-todo".into()));
        assert!(personal_slugs.contains(&"shared-readme".into()));
        assert!(
            !personal_slugs.contains(&"work-internal".into()),
            "personal profile must filter out work-* per its own glob"
        );

        let work_reg = adapter.build_skills_registry_for(work.as_ref());
        let work_slugs: Vec<String> = work_reg.list().iter().map(|s| s.slug.to_string()).collect();
        assert!(work_slugs.contains(&"work-internal".into()));
        assert!(work_slugs.contains(&"shared-readme".into()));
        assert!(
            !work_slugs.contains(&"personal-todo".into()),
            "work profile must filter out personal-* per its own glob (NOT inherit personal's filter)"
        );

        let empty_reg = adapter.build_skills_registry_for(no_skills.as_ref());
        assert_eq!(
            empty_reg.list().len(),
            0,
            "skills = [] explicit off-switch yields empty registry"
        );
    }

    /// `instance_skills` returns `None` for a key that isn't live;
    /// `focused_skills` returns `None` when the registry is empty.
    /// These are the safety nets the palette relies on to render an
    /// empty list rather than panic when no instance has spawned.
    #[tokio::test]
    async fn instance_and_focused_skills_return_none_when_empty() {
        let adapter = AcpAdapter::new(Config::default(), Arc::new(StatusBroadcast::new(true)));
        let bogus = InstanceKey::parse("550e8400-e29b-41d4-a716-446655440000").unwrap();
        assert!(adapter.instance_skills(bogus).await.is_none());
        assert!(adapter.focused_skills().await.is_none());
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
        let err = adapter
            .restart_instance(Some(key), None, false, None, None)
            .await
            .expect_err("unknown id must fail");
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
[profile]
default = "ask"

[[agents]]
id = "claude-code"
provider = "acp-claude-code"
command = "bunx"

[[profiles]]
id = "ask"
agent = "claude-code"

[[profiles]]
id = "strict"
agent = "claude-code"
"#,
        )
        .expect("parses");
        let adapter = AcpAdapter::new(cfg, Arc::new(StatusBroadcast::new(true)));
        let out = adapter.list_profiles();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].id, "ask");
        assert_eq!(out[0].agent, "claude-code");
        assert!(out[0].is_default);
        assert_eq!(out[1].id, "strict");
        assert!(!out[1].is_default);
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
