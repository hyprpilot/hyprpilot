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
    /// Daemon-side "needs attention" tracker. Listener task wired in
    /// [`Self::spawn_tauri_event_bridge`] subscribes to the registry's
    /// broadcast and raises / clears entries per the policy in
    /// [`crate::adapters::notifications`]. `submit_prompt` calls
    /// `clear` directly so a captain engaging via prompt drops the
    /// row without waiting for the broadcast round-trip.
    notifications: Arc<crate::adapters::notifications::Notifications>,
    /// Slash-commands cache shared with the composer-autocomplete
    /// `CommandsSource`. Daemon installs it once at boot via
    /// [`Self::set_commands_cache`]; per-instance runtimes write to
    /// it on every `available_commands_update` notification.
    commands_cache: Arc<RwLock<Option<crate::completion::source::commands::CommandsCache>>>,
    /// Captain's currently-selected default profile id. Daemon-side
    /// singleton: every frontend (Vue overlay, nvim plugin, ctl)
    /// reads + writes through here so cross-frontend selections stay
    /// in sync. Seeded at construction from `config.profile.default`;
    /// runtime mutations via `profile/set` are in-memory only — a
    /// daemon restart re-reads from config. Mutation publishes
    /// `acp:profile-changed` so passive consumers refresh without
    /// polling.
    selected_profile_id: RwLock<Option<String>>,
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
        // Seed the daemon-singleton selection from `[profile] default`
        // at construction. Config validation (`validate_profiles_non_empty`
        // + the cross-field check that `[profile] default` references a
        // real `[[profiles]]` id) already ran at boot, so this clone is
        // either the captain's chosen default or `None` (no profiles
        // configured — daemon spawn paths reject before they reach the
        // singleton, this leaves the slot empty until config gains
        // entries).
        let initial_profile = config.read().map(|cfg| cfg.profile.default.clone()).unwrap_or(None);
        let registry = Arc::new(AdapterRegistry::new());
        let notifications = Arc::new(crate::adapters::notifications::Notifications::new(registry.events_tx()));
        Self {
            config,
            status,
            registry,
            permissions,
            busy_instances: Arc::new(RwLock::new(HashSet::new())),
            commands_cache: Arc::new(RwLock::new(None)),
            selected_profile_id: RwLock::new(initial_profile),
            notifications,
        }
    }

    /// Notifications tracker handle. Snapshot reads (boot snapshot,
    /// `notifications/list`, `notifications_list` Tauri command) and
    /// the `submit_prompt` clear-on-engage path go through this.
    #[must_use]
    pub fn notifications(&self) -> &Arc<crate::adapters::notifications::Notifications> {
        &self.notifications
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

    /// Per-instance MCP catalog as a flat `Vec<MCPDefinition>`. Drives
    /// the `mcps_list` Tauri command's preview pane AND backs the
    /// header chrome's `+N mcps` pill via the daemon mirror's
    /// `MetaSnapshot.mcpsCount` (the actor's `MetaEmitter` builds its
    /// count off the same registry).
    ///
    /// Resolution mirrors the spawn-time `build_mcp_registry_with` so
    /// the palette sees:
    ///   1. **Root `[[patches]]` mcps**, folded onto the profile via
    ///      `resolve_effective_profile`.
    ///   2. **`--with-config` patches** the captain supplied at spawn
    ///      time, pulled off the live instance handle's
    ///      `config_patches`. Without this the palette saw the base
    ///      profile's mcps while the actor ran with per-invocation
    ///      overlays — the captain's spawn-time additions were
    ///      invisible in the UI.
    ///   3. **In-tree auto-injected `hyprpilot mcp serve`** server when
    ///      the resolved `[mcp]` block has `enabled = true` AND the
    ///      skills registry is non-empty. Without this the auto-inject
    ///      ran on the wire but the captain saw no entry for it in
    ///      the palette / header count.
    pub async fn resolve_mcp_catalog(&self, instance_id: Option<&str>) -> Vec<crate::mcp::MCPDefinition> {
        let key = instance_id.and_then(|id| InstanceKey::parse(id).ok());
        let handle = match key {
            Some(k) => self.registry.get(k).await,
            None => None,
        };
        // `AcpInstance.profile_id` is already `Option<String>`; the
        // outer Option here would be `Option<Option<String>>` from a
        // naive `Some(h.profile_id.clone())`. `.and_then(|p| p)`
        // flattens to `Option<String>` so `as_deref()` works
        // downstream.
        let profile_id: Option<String> = handle.as_ref().and_then(|h| h.profile_id.clone());
        // Pull the live instance's `--with-config` overlays. None when
        // there's no live instance OR when the spawn didn't carry
        // any. Empty slice short-circuits in
        // `resolve_effective_profile` so the no-overlay path stays
        // cheap.
        let external_patches = match &handle {
            Some(h) => h.config_patches.clone(),
            None => Vec::new(),
        };
        let skills_arc = handle.as_ref().map(|h| h.skills.clone());
        let cfg = self.read_config().clone();
        let profile = match resolve_effective_profile(&cfg, profile_id.as_deref(), &external_patches) {
            Ok(p) => p,
            Err(_) => return Vec::new(),
        };

        let mcp_cfg = effective_mcp_with(&profile);
        let mut defs = crate::mcp::loader::load_files(&effective_mcp_files_with(&profile));
        apply_mcp_glob_defaults(&mut defs, &mcp_cfg);

        // Add the auto-injected `hyprpilot mcp serve` server at the
        // head, same as `build_mcp_registry_with`. Match its
        // condition: `[mcp].enabled` true AND a non-empty skills
        // registry to project. Pre-spawn (no live handle) we don't
        // have a skills registry to feed the auto-inject builder, so
        // the catalog elides the entry — matches the
        // pre-`session/new` view where no actor exists.
        if let Some(skills) = skills_arc {
            if mcp_cfg.enabled() {
                if let Some(auto) = crate::mcp::auto_inject::build_auto_inject_definition(
                    &skills,
                    &mcp_cfg,
                    std::path::PathBuf::from("<auto-injected:hyprpilot mcp serve>"),
                ) {
                    defs.insert(0, auto);
                }
            }
        }
        defs
    }

    /// Build the per-instance `SkillsRegistry` from the resolved
    /// entries. Calls `reload()` immediately so the registry is
    /// ready-to-list at spawn time (no first-prompt lag while disk
    /// walks). Empty registries are valid — captain may have
    /// `skills = []` set explicitly. `reload()` errors are logged
    /// and swallowed; the captain can hit `skills/reload` to retry.
    /// Test harness only — production paths use the
    /// `build_skills_registry_with` free function so the
    /// `--with-config` patched-config path can pass an explicit
    /// `&Config`.
    #[cfg(test)]
    fn build_skills_registry_for(&self, profile: &crate::config::ProfileConfig) -> Arc<crate::skills::SkillsRegistry> {
        build_skills_registry_with(profile)
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

    /// Read the daemon-singleton selected profile id. `None` only when
    /// `[profile] default` was unset at config-load AND no client has
    /// called `profile/set` since.
    #[must_use]
    pub fn selected_profile_id(&self) -> Option<String> {
        self.selected_profile_id
            .read()
            .expect("selected_profile_id lock poisoned")
            .clone()
    }

    /// Resolve the cwd a fresh spawn would land in under the addressed
    /// profile, with root `[[patches]]` folded onto the base — same
    /// resolver the spawn path uses (`resolve_effective_profile`),
    /// just exposed for the UI's pre-spawn preview. `None` when the
    /// resolved profile has no cwd set after patches (the UI then
    /// falls back to the daemon's process cwd).
    ///
    /// `profile_id = None` falls through to the daemon-singleton
    /// selected profile, then `[profile] default`. Output is
    /// display-formatted (`$HOME → ~`).
    pub fn resolve_spawn_cwd(&self, profile_id: Option<&str>) -> Result<Option<String>, RpcError> {
        let cfg = self.read_config();
        let runtime_default = self.selected_profile_id();
        let effective_profile_id = profile_id.or(runtime_default.as_deref());
        let patched = resolve_effective_profile(&cfg, effective_profile_id, &[])?;
        Ok(patched
            .cwd
            .as_ref()
            .map(|p| crate::tools::path::display_cwd(&p.to_string_lossy())))
    }

    /// Mutate the daemon-singleton selected profile. Validates against
    /// the loaded `[[profiles]]` registry — unknown ids reject with
    /// `-32602 invalid_params` consistent with the spawn path.
    /// Publishes `acp:profile-changed` on success so every frontend
    /// syncs without polling.
    pub fn set_selected_profile_id(&self, profile_id: &str) -> Result<Value, RpcError> {
        {
            let cfg = self.read_config();
            if !cfg.profiles.iter().any(|p| p.id == profile_id) {
                return Err(RpcError::invalid_params(format!(
                    "profile '{profile_id}' is not in the [[profiles]] registry"
                )));
            }
        }
        {
            let mut w = self
                .selected_profile_id
                .write()
                .expect("selected_profile_id lock poisoned");
            *w = Some(profile_id.to_string());
        }
        let _ = self
            .registry
            .events_tx()
            .send(crate::adapters::InstanceEvent::SelectedProfileChanged {
                profile_id: profile_id.to_string(),
            });
        Ok(serde_json::json!({ "profileId": profile_id }))
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
    /// Resolve a `(ResolvedInstance, effective_profile)` pair against
    /// the daemon's current `Config`, folding any `withConfig` patches
    /// onto the **resolved profile** (not the root config). The patch
    /// shape mirrors `[[profiles]]` in TOML; patches can override
    /// `model`, `mode`, `system_prompt`, `mcps`, `skills`, `env`,
    /// `cwd`, or even swap the underlying `agent` to point at a
    /// different entry in the (unpatched) agent
    /// registry. Root-level knobs (theme, daemon.window, the agent
    /// registry itself) are deliberately out of scope — they belong
    /// in the on-disk config or a `daemon/reload`.
    ///
    /// When neither `--profile` nor `[profile] default` addresses a
    /// real `[[profiles]]` entry, resolution errors — every spawn
    /// flows through a profile (no bare-agent fallback), so
    /// `withConfig` always has a typed `ProfileConfig` to fold onto.
    ///
    /// Returns the patched `ProfileConfig` alongside the
    /// `ResolvedInstance`. Downstream `build_mcp_registry_with` /
    /// `build_skills_registry_with` reads from this one shape so
    /// the captain's patches reach every consumer (the bug pre-hoist
    /// was the MCP / skills registries getting the UNPATCHED profile
    /// while `ResolvedInstance` got the patched one — silent drift).
    /// Errors surface as `-32602 invalid_params` with the serde /
    /// garde report inline.
    pub(crate) fn resolve_with_patches(
        &self,
        agent_id: Option<&str>,
        profile_id: Option<&str>,
        patches: &[Value],
    ) -> Result<(ResolvedInstance, ProfileConfig), RpcError> {
        let cfg = self.read_config().clone();
        // Caller-supplied profile_id wins; otherwise fall back to the
        // daemon-singleton runtime selection (mutable via `profile/set`).
        // The inner `base_profile_for_patches` falls further through to
        // `[profile] default` when both are unset.
        let runtime_default = self.selected_profile_id();
        let effective_profile_id = profile_id.or(runtime_default.as_deref());
        let result = resolve_into_instance_and_profile(&cfg, agent_id, effective_profile_id, patches)?;
        tracing::debug!(
            patch_count = patches.len(),
            root_patch_count = cfg.patches.as_deref().map_or(0, <[_]>::len),
            profile_id = %result.1.id,
            "acp::adapter: root [[patches]] + withConfig patches applied to resolved profile"
        );
        Ok(result)
    }

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

        // Notifications listener — owns the "needs attention" tracker
        // off the same registry broadcast. Seeded with `focused_id =
        // None` because this runs in the Tauri `setup(...)` closure
        // before any instance has spawned; the listener picks up the
        // first focus pointer the moment `InstancesFocused` fires
        // (auto-focus on first spawn is part of the registry contract).
        let notifications = self.notifications.clone();
        let notifications_rx = self.registry.subscribe();
        crate::adapters::notifications::spawn_listener(notifications, notifications_rx, None);

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
        // PermissionRequest → Awaiting; PermissionResolved while a turn
        // is still open → back to Streaming (without this, waybar
        // stuck on Awaiting until TurnEnded fired); TurnEnded → Idle
        // (or Error when the turn carried an error); a crash-path
        // `State::Ended | Error` clears stale Awaiting / Streaming on
        // the instance that just dropped.
        //
        // `open_session` tracks the live turn's session id so the
        // PermissionResolved revert can re-emit Streaming with the
        // correct active_session field; without it the revert would
        // null out active_session and waybar would lose context.
        let mut status_rx = self.registry.subscribe();
        let status = self.status.clone();
        tauri::async_runtime::spawn(async move {
            let mut open_session: Option<String> = None;

            loop {
                match status_rx.recv().await {
                    Ok(InstanceEvent::TurnStarted { session_id, .. }) => {
                        open_session = Some(session_id.clone());
                        status.set_state(crate::rpc::protocol::AgentState::Streaming, Some(session_id));
                    }
                    Ok(InstanceEvent::PermissionRequest { session_id, .. }) => {
                        status.set_state(crate::rpc::protocol::AgentState::Awaiting, Some(session_id));
                    }
                    Ok(InstanceEvent::PermissionResolved { .. }) => {
                        // Captain answered (or it timed out). If a turn is
                        // still open the agent has work left — flip back
                        // to Streaming so waybar reflects "running" the
                        // instant the answer lands, instead of waiting
                        // for the next chunk / TurnEnded to reconcile.
                        if let Some(session_id) = open_session.clone() {
                            status.set_state(crate::rpc::protocol::AgentState::Streaming, Some(session_id));
                        }
                    }
                    Ok(InstanceEvent::TurnEnded { error, .. }) => {
                        open_session = None;
                        let next = if error.is_some() {
                            crate::rpc::protocol::AgentState::Error
                        } else {
                            crate::rpc::protocol::AgentState::Idle
                        };
                        status.set_state(next, None);
                    }
                    Ok(InstanceEvent::State {
                        state: InstanceState::Ended | InstanceState::Error,
                        ..
                    }) => {
                        // Defensive: an actor that ended mid-turn (crash,
                        // SIGTERM during a prompt) might never emit
                        // TurnEnded; without this the status sticks on
                        // Streaming / Awaiting forever from waybar's POV.
                        open_session = None;
                        status.set_state(crate::rpc::protocol::AgentState::Idle, None);
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

    /// Resolve a `(agent_id?, profile_id?)` pair into the spawn-time
    /// pair every downstream consumer reads from: the
    /// `ResolvedInstance` (agent + system_prompt + model + mode) AND
    /// the post-patches `ProfileConfig` (mcps + mcp.skills + cwd +
    /// env). Returning both at once eliminates the drift that bit
    /// us pre-hoist where `ensure()` re-fetched the UNPATCHED
    /// profile via `profile_by_id_in` and the MCP / skills
    /// registries silently dropped root `[[patches]]` content.
    ///
    /// Errors when neither `--profile` nor `[profile] default`
    /// addresses a real `[[profiles]]` entry — every spawn flows
    /// through a profile. Explicit `agent_id` overrides whatever
    /// agent the patched profile names (same profile, new agent
    /// spawn).
    fn resolve(
        &self,
        agent_id: Option<&str>,
        profile_id: Option<&str>,
    ) -> Result<(ResolvedInstance, ProfileConfig), RpcError> {
        let cfg = self.read_config();
        // Caller-supplied profile_id wins; otherwise fall back to the
        // daemon-singleton runtime selection (mutable via `profile/set`).
        // The inner `base_profile_for_patches` falls further through to
        // `[profile] default` when both are unset.
        let runtime_default = self.selected_profile_id();
        let effective_profile_id = profile_id.or(runtime_default.as_deref());
        resolve_into_instance_and_profile(&cfg, agent_id, effective_profile_id, &[])
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
        effective_profile: ProfileConfig,
        bootstrap: Bootstrap,
    ) -> Result<InstanceKey, RpcError> {
        self.ensure_with_config(key, resolved, bootstrap, effective_profile, Vec::new())
            .await
    }

    /// Variant of [`Self::ensure`] that also stores the
    /// `--with-config` patch list on the spawned instance for
    /// `restart_instance` to replay against whatever config the
    /// daemon currently has.
    async fn ensure_with_config(
        &self,
        key: InstanceKey,
        resolved: ResolvedInstance,
        bootstrap: Bootstrap,
        effective_profile: ProfileConfig,
        config_patches: Vec<Value>,
    ) -> Result<InstanceKey, RpcError> {
        let replace_existing = matches!(bootstrap, Bootstrap::Resume(_));
        if !replace_existing && self.registry.get(key).await.is_some() {
            return Ok(key);
        }
        if replace_existing {
            let _ = self.registry.shutdown_one(key).await;
        }

        // `effective_profile` is already fully patched (root
        // `[[patches]]` + any `--with-config` overlays applied
        // upstream in `resolve_effective_profile`). The MCP /
        // skills registries below read straight from it — there's
        // no longer a root-level fallback layer to consult.
        let profile = effective_profile;
        let profile_id = resolved.profile_id.clone();
        // Per-instance MCP catalog reads from the patched profile
        // (root `[[patches]]` + any `--with-config` overlays are
        // already folded). `DecisionContext.mcps` consumes the
        // resulting registry at `PermissionController::decide` lane
        // 2. `None` from the builder means no MCP files wired — the
        // per-server lane short-circuits and every call falls
        // through to AskUser (or trust store).
        let skills = build_skills_registry_with(&profile);
        let mcps = build_mcp_registry_with(&profile, Some(&skills));
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
            config_patches,
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
        let (resolved, effective_profile) = self.resolve(agent_id, profile_id)?;

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

        let key = self.ensure(key, resolved, effective_profile, Bootstrap::Fresh).await?;

        // Captain is engaging with this instance — drop any pending
        // "needs attention" entry. Without this clear path the entry
        // would linger until the next `InstancesFocused` event landed,
        // which on a remote captain who sends a prompt without
        // explicitly focusing is "never".
        self.notifications.clear(&key.as_string());

        // Check busy state BEFORE submitting so the wire reply can
        // tell the caller whether the actor was already mid-turn
        // when their prompt landed. The actor's `cmd_rx` is an
        // unbounded mpsc — a prompt sent while a turn is active
        // sits in the channel until the current turn drains; the
        // captain's frontend may want to render "queued behind
        // running turn" UI in that window. `busy_instance_ids` is
        // maintained off the `TurnStarted`/`TurnEnded` broadcast
        // (see `spawn_busy_tracker`), so this read is a snapshot
        // — by the time the actor reads the new command the turn
        // may have ended, but the disposition reply still reflects
        // the moment-in-time the captain submitted.
        let was_busy = self.busy_instance_ids().await.iter().any(|id| id == &key.as_string());

        let handle = self
            .registry
            .get(key)
            .await
            .ok_or_else(|| RpcError::internal_error("instance actor vanished before accepting prompt"))?;
        let cmd_tx = handle.cmd_tx.clone();

        let (reply_tx, reply_rx) = oneshot::channel();
        cmd_tx
            .send(InstanceCommand::Prompt {
                text: text.to_string(),
                attachments: attachments.to_vec(),
                // External `prompts/send` always honours the queue
                // auto-route — if a turn is mid-flight the captain's
                // prompt lands in the visible queue, not as a parallel
                // dispatch.
                force_dispatch: false,
                reply: reply_tx,
            })
            .map_err(|_| RpcError::internal_error(prompt_actor_closed_message(&handle)))?;

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
            "disposition": if was_busy { "queued" } else { "sent" },
            "wasBusy": was_busy,
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
            let (resolved, _) = self.resolve(agent_id, None)?;
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

    /// Resolve a captain-supplied `instance_id` (optional) into a live
    /// actor's command channel. Falls back to the focused instance
    /// when omitted (same convention as `prompts/cancel` /
    /// `permissions/respond`). Returns `None` when neither resolves.
    async fn resolve_queue_cmd_tx(
        &self,
        instance_id: Option<&str>,
    ) -> Option<tokio::sync::mpsc::UnboundedSender<InstanceCommand>> {
        let key = match instance_id {
            Some(s) => InstanceKey::parse(s).ok()?,
            None => self.registry.focused_id().await?,
        };
        self.registry.get(key).await.map(|h| h.cmd_tx.clone())
    }

    /// Read the current queue. Empty `Vec` for an unknown instance
    /// id (the RPC handler reports back to the captain; the facade
    /// keeps the shape uniform).
    pub async fn queue_list(
        &self,
        instance_id: Option<&str>,
    ) -> Result<Vec<crate::adapters::queue::QueueItem>, RpcError> {
        let Some(cmd_tx) = self.resolve_queue_cmd_tx(instance_id).await else {
            return Ok(Vec::new());
        };
        let (reply_tx, reply_rx) = oneshot::channel();
        cmd_tx
            .send(InstanceCommand::QueueList { reply: reply_tx })
            .map_err(|_| RpcError::internal_error("queue/list: actor channel closed"))?;
        reply_rx
            .await
            .map_err(|_| RpcError::internal_error("queue/list: actor dropped reply"))
    }

    /// In-place edit. Text always replaces; `attachments` replaces
    /// only when supplied. Errors when the item id is unknown.
    pub async fn queue_edit(
        &self,
        instance_id: Option<&str>,
        item_id: String,
        text: String,
        attachments: Option<Vec<crate::adapters::Attachment>>,
    ) -> Result<crate::adapters::queue::QueueItem, RpcError> {
        let Some(cmd_tx) = self.resolve_queue_cmd_tx(instance_id).await else {
            return Err(RpcError::invalid_params("queue/edit: no live instance"));
        };
        let (reply_tx, reply_rx) = oneshot::channel();
        cmd_tx
            .send(InstanceCommand::QueueEdit {
                item_id,
                text,
                attachments,
                reply: reply_tx,
            })
            .map_err(|_| RpcError::internal_error("queue/edit: actor channel closed"))?;
        reply_rx
            .await
            .map_err(|_| RpcError::internal_error("queue/edit: actor dropped reply"))?
            .map_err(RpcError::invalid_params)
    }

    pub async fn queue_remove(&self, instance_id: Option<&str>, item_id: String) -> Result<bool, RpcError> {
        let Some(cmd_tx) = self.resolve_queue_cmd_tx(instance_id).await else {
            return Ok(false);
        };
        let (reply_tx, reply_rx) = oneshot::channel();
        cmd_tx
            .send(InstanceCommand::QueueRemove {
                item_id,
                reply: reply_tx,
            })
            .map_err(|_| RpcError::internal_error("queue/remove: actor channel closed"))?;
        reply_rx
            .await
            .map_err(|_| RpcError::internal_error("queue/remove: actor dropped reply"))?
            .map_err(RpcError::internal_error)
    }

    pub async fn queue_move(
        &self,
        instance_id: Option<&str>,
        item_id: String,
        position: usize,
    ) -> Result<bool, RpcError> {
        let Some(cmd_tx) = self.resolve_queue_cmd_tx(instance_id).await else {
            return Ok(false);
        };
        let (reply_tx, reply_rx) = oneshot::channel();
        cmd_tx
            .send(InstanceCommand::QueueMove {
                item_id,
                position,
                reply: reply_tx,
            })
            .map_err(|_| RpcError::internal_error("queue/move: actor channel closed"))?;
        reply_rx
            .await
            .map_err(|_| RpcError::internal_error("queue/move: actor dropped reply"))?
            .map_err(RpcError::internal_error)
    }

    pub async fn queue_clear(&self, instance_id: Option<&str>) -> Result<u32, RpcError> {
        let Some(cmd_tx) = self.resolve_queue_cmd_tx(instance_id).await else {
            return Ok(0);
        };
        let (reply_tx, reply_rx) = oneshot::channel();
        cmd_tx
            .send(InstanceCommand::QueueClear { reply: reply_tx })
            .map_err(|_| RpcError::internal_error("queue/clear: actor channel closed"))?;
        reply_rx
            .await
            .map_err(|_| RpcError::internal_error("queue/clear: actor dropped reply"))?
            .map_err(RpcError::internal_error)
    }

    /// Pop the named item (or the head when `None`) AND dispatch it
    /// immediately, regardless of busy. ACP serialises on the wire so
    /// a concurrent active turn is fine — the popped item chains
    /// behind. Captain's "send now" intent.
    pub async fn queue_dispatch(
        &self,
        instance_id: Option<&str>,
        item_id: Option<String>,
    ) -> Result<crate::adapters::queue::QueueDispatchResult, RpcError> {
        let Some(cmd_tx) = self.resolve_queue_cmd_tx(instance_id).await else {
            return Err(RpcError::invalid_params("queue/dispatch: no live instance"));
        };
        let (reply_tx, reply_rx) = oneshot::channel();
        cmd_tx
            .send(InstanceCommand::QueueDispatch {
                item_id,
                reply: reply_tx,
            })
            .map_err(|_| RpcError::internal_error("queue/dispatch: actor channel closed"))?;
        reply_rx
            .await
            .map_err(|_| RpcError::internal_error("queue/dispatch: actor dropped reply"))?
            .map_err(RpcError::internal_error)
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
            let (resolved, _profile) = self.resolve(agent_id, profile_id)?;
            let ephemeral_key = key.unwrap_or_else(InstanceKey::new_v4);
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
                config_patches: Vec::new(),
            });
            let tx = instance.cmd_tx.clone();
            (tx, Some(instance))
        };

        let (reply_tx, reply_rx) = oneshot::channel();
        cmd_tx
            .send(InstanceCommand::ListSessions { cwd, reply: reply_tx })
            .map_err(|_| {
                let summary = ephemeral
                    .as_ref()
                    .map(list_actor_closed_summary)
                    .unwrap_or_else(|| "actor closed".into());
                RpcError::internal_error(format!(
                    "instance actor closed before accepting list request: {summary}"
                ))
            })?;

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

            // Order by `updatedAt` descending — most-recently-used
            // session surfaces first across every consumer (sessions/list
            // RPC, `ctl sessions list`, the command palette's session
            // picker). ACP ships ISO-8601 timestamps which sort
            // lexically. `None` → empty string → sorts last; ties keep
            // insertion order via `sort_by`'s stability.
            r.sessions.sort_by(|a, b| {
                let au = a.updated_at.as_deref().unwrap_or("");
                let bu = b.updated_at.as_deref().unwrap_or("");
                bu.cmp(au)
            });
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
        config_patches: Vec<Value>,
    ) -> Result<InstanceKey, RpcError> {
        let key = match instance_id {
            Some(s) => InstanceKey::parse(s).map_err(map_adapter_error_to_rpc)?,
            None => InstanceKey::new_v4(),
        };
        // Apply `--with-config` overlays before resolving. Mirrors
        // `spawn_instance`: the captain's overlays patch the
        // resolved profile (model / mode / mcps / skills /
        // system_prompt / env / cwd) and the
        // patches get stored on the resumed instance so a subsequent
        // `instances/restart` replays them — same semantics as a
        // Fresh-spawned instance.
        let (mut resolved, effective_profile) = self.resolve_with_patches(agent_id, profile_id, &config_patches)?;
        // Override the profile-default cwd with the session's own. ACP
        // agents (claude-agent-acp) scope persisted sessions BY cwd —
        // resuming session-X under any cwd other than the one it was
        // created with returns "Resource not found". The UI knows the
        // session's cwd from `session_list`; thread it through here so
        // the resume request lands in the right scope.
        if let Some(c) = cwd {
            resolved.agent.cwd = Some(c);
        }
        self.ensure_with_config(
            key,
            resolved,
            Bootstrap::Resume(session_id),
            effective_profile,
            config_patches,
        )
        .await?;
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
    /// before spawn. When `spec.config_patches` is non-empty, the
    /// patches are folded onto a clone of the daemon's `Config`
    /// before resolution — the resulting tree drives this one spawn
    /// only (and is stored on the instance so `restart_instance`
    /// replays it).
    pub async fn spawn_instance(&self, spec: SpawnSpec) -> Result<InstanceKey, RpcError> {
        let SpawnSpec {
            profile_id,
            agent_id,
            cwd,
            mode,
            model,
            config_patches,
        } = spec;
        let (mut resolved, effective_profile) =
            self.resolve_with_patches(agent_id.as_deref(), profile_id.as_deref(), &config_patches)?;
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
        self.ensure_with_config(key, resolved, Bootstrap::Fresh, effective_profile, config_patches)
            .await
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
                    config_patches: Vec::new(),
                };
                let (new_key, _handle) = self.resolve_or_spawn(key, ensure, spec).await?;
                return Ok(new_key);
            }
        };

        let existing_agent_id = existing.agent_id.clone();
        let existing_profile_id = existing.profile_id.clone();
        let mode = existing.mode.clone();
        // Carry the captain's original `--with-config` patches so the
        // restart preserves the effective config the instance was
        // born with. Re-applies against the daemon's current `Config`
        // so a `daemon/reload` between spawn and restart picks up
        // the new base while keeping the overlays.
        let config_patches = existing.config_patches.clone();
        drop(existing);

        let slot = self
            .registry
            .drop_preserving_slot(key)
            .await
            .map_err(map_adapter_error_to_rpc)?;

        let (mut resolved, effective_profile) = self.resolve_with_patches(
            Some(&existing_agent_id),
            existing_profile_id.as_deref(),
            &config_patches,
        )?;
        if mode.is_some() {
            resolved.mode = mode;
        }
        if let Some(c) = cwd {
            resolved.agent.cwd = Some(c);
        }
        let profile_id_for_instance = resolved.profile_id.clone();
        let skills = build_skills_registry_with(&effective_profile);
        let mcps = build_mcp_registry_with(&effective_profile, Some(&skills));
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
            config_patches,
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
        // `is_default` follows the agent referenced by `[profile]
        // default` — there is no standalone `[agent] default`
        // anymore. When `[profile] default` is unset or names a
        // non-existent profile, no agent gets the badge.
        let default_agent = cfg
            .profile
            .default
            .as_deref()
            .and_then(|id| cfg.profiles.iter().find(|p| p.id == id))
            .map(|p| p.agent.as_str());
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
    ///
    /// **Resolution applies root `[[patches]]`** so frontends see the
    /// same shape the spawn path will produce. Captains commonly put
    /// shared values (`cwd`, `system_prompt`, `mcps`) in root patches
    /// scoped by `$match.profile`; reading `cfg.profiles` raw would
    /// surface the unpatched values + leave header chrome pre-seeds
    /// stale until the actor's `session/new` lands and overrides.
    /// Captain reported `cwd` pre-spawn seed broken because their
    /// profile's cwd lives in a root patch, not directly on the
    /// profile entry.
    pub fn list_profiles(&self) -> Vec<crate::adapters::ProfileSummary> {
        let cfg = self.read_config();
        let default_profile = cfg.profile.default.as_deref();
        cfg.profiles
            .iter()
            .map(|p| {
                // Apply root patches to each profile so the summary
                // mirrors what `resolve_effective_profile` would
                // produce at spawn-time (minus per-spawn
                // `--with-config` overlays, which can't be summarised
                // ahead of time). Falls back to the raw entry on any
                // patch / validation failure — readonly listing
                // shouldn't surface a config error.
                let resolved = resolve_effective_profile(&cfg, Some(p.id.as_str()), &[]).unwrap_or_else(|_| p.clone());
                crate::adapters::ProfileSummary {
                    id: resolved.id.clone(),
                    agent: resolved.agent.clone(),
                    model: resolved.model.clone(),
                    // Ship the raw configured cwd (no `~` expansion).
                    // The spawn path canonicalises when it actually
                    // launches the agent.
                    cwd: resolved.cwd.as_ref().map(|c| c.display().to_string()),
                    is_default: default_profile == Some(p.id.as_str()),
                }
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
        let (mut resolved, effective_profile) = self.resolve(spec.agent_id.as_deref(), spec.profile_id.as_deref())?;
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
        let new_key = self
            .ensure(new_key, resolved, effective_profile, Bootstrap::Fresh)
            .await?;
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
            config_patches: Vec::new(),
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

    fn notifications(&self) -> Option<std::sync::Arc<crate::adapters::notifications::Notifications>> {
        Some(self.notifications.clone())
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

    fn selected_profile_id(&self) -> Option<String> {
        AcpAdapter::selected_profile_id(self)
    }

    fn set_selected_profile_id(&self, profile_id: &str) -> AdapterResult<serde_json::Value> {
        AcpAdapter::set_selected_profile_id(self, profile_id).map_err(rpc_to_adapter)
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
        config_patches: Vec<Value>,
    ) -> AdapterResult<InstanceKey> {
        AcpAdapter::load_session(self, instance_id, agent_id, profile_id, session_id, cwd, config_patches)
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

    // ── queue/* dispatch ─────────────────────────────────────────

    async fn queue_list(&self, instance_id: Option<&str>) -> AdapterResult<Vec<crate::adapters::queue::QueueItem>> {
        AcpAdapter::queue_list(self, instance_id).await.map_err(rpc_to_adapter)
    }

    async fn queue_edit(
        &self,
        instance_id: Option<&str>,
        item_id: String,
        text: String,
        attachments: Option<Vec<crate::adapters::transcript::Attachment>>,
    ) -> AdapterResult<crate::adapters::queue::QueueItem> {
        AcpAdapter::queue_edit(self, instance_id, item_id, text, attachments)
            .await
            .map_err(rpc_to_adapter)
    }

    async fn queue_remove(&self, instance_id: Option<&str>, item_id: String) -> AdapterResult<bool> {
        AcpAdapter::queue_remove(self, instance_id, item_id)
            .await
            .map_err(rpc_to_adapter)
    }

    async fn queue_move(&self, instance_id: Option<&str>, item_id: String, position: usize) -> AdapterResult<bool> {
        AcpAdapter::queue_move(self, instance_id, item_id, position)
            .await
            .map_err(rpc_to_adapter)
    }

    async fn queue_clear(&self, instance_id: Option<&str>) -> AdapterResult<u32> {
        AcpAdapter::queue_clear(self, instance_id).await.map_err(rpc_to_adapter)
    }

    async fn queue_dispatch(
        &self,
        instance_id: Option<&str>,
        item_id: Option<String>,
    ) -> AdapterResult<crate::adapters::queue::QueueDispatchResult> {
        AcpAdapter::queue_dispatch(self, instance_id, item_id)
            .await
            .map_err(rpc_to_adapter)
    }
}

/// Project the patched profile's `mcps` field onto the resolved
/// runtime shape. Reads ONLY from the patched profile — root
/// `[[patches]]` were folded onto it in `resolve_effective_profile`
/// upstream.
fn effective_mcp_files_with(profile: &ProfileConfig) -> Vec<crate::config::ResolvedMcpFile> {
    profile
        .mcps
        .as_deref()
        .map(|files| files.iter().map(crate::config::ResolvedMcpFile::from_entry).collect())
        .unwrap_or_default()
}

/// Build the per-instance MCP registry from the patched profile.
///
/// Prepends an **auto-injected** entry for the in-tree `hyprpilot mcp
/// serve` server when the resolved `[mcp]` block has `enabled = true`
/// AND the per-instance skills registry (after applying the optional
/// slug whitelist) is non-empty. The daemon's resolved skill set
/// rides through to the agent vendor as a stdio MCP server it spawns
/// itself. Auto-inject is independent of user-declared `mcps` —
/// `mcps = []` does not suppress the in-tree server (that's what
/// `mcp.enabled = false` is for).
fn build_mcp_registry_with(
    profile: &ProfileConfig,
    skills: Option<&Arc<crate::skills::SkillsRegistry>>,
) -> Option<Arc<crate::mcp::MCPsRegistry>> {
    let mcp_cfg = effective_mcp_with(profile);
    let files = effective_mcp_files_with(profile);
    let mut defs = crate::mcp::loader::load_files(&files);
    apply_mcp_glob_defaults(&mut defs, &mcp_cfg);

    // Auto-inject only when the effective [mcp] block opts in AND
    // there's a non-empty skills registry to project. Source is a
    // synthetic path so the UI's "which file owns this server"
    // surfaces a recognisable label.
    if let Some(skills_arc) = skills {
        if mcp_cfg.enabled() {
            if let Some(auto) = crate::mcp::auto_inject::build_auto_inject_definition(
                skills_arc,
                &mcp_cfg,
                std::path::PathBuf::from("<auto-injected:hyprpilot mcp serve>"),
            ) {
                defs.insert(0, auto);
            }
        }
    }

    if defs.is_empty() {
        return None;
    }
    Some(Arc::new(crate::mcp::MCPsRegistry::new(defs)))
}

fn apply_mcp_glob_defaults(defs: &mut [crate::mcp::MCPDefinition], cfg: &crate::config::McpConfig) {
    for def in defs {
        if def.hyprpilot.auto_accept_tools.is_empty() {
            def.hyprpilot.auto_accept_tools = cfg.auto_accept_tools().to_vec();
        }

        if def.hyprpilot.auto_reject_tools.is_empty() {
            def.hyprpilot.auto_reject_tools = cfg.auto_reject_tools().to_vec();
        }
    }
}

/// Resolved `[mcp]` block for an instance — reads ONLY from the
/// patched profile. Falls back to the typed `Default::default()`
/// when the profile has no `mcp` block (enabled=true,
/// autoAcceptTools=["*"], no skills).
fn effective_mcp_with(profile: &ProfileConfig) -> crate::config::McpConfig {
    profile.mcp.clone().unwrap_or_default()
}

/// Skills slugs the auto-injected `hyprpilot` MCP server should
/// expose for this instance. Reads from the patched profile's
/// `mcp.skills` (root `[[patches]]` already folded upstream).
fn effective_skills_with(profile: &ProfileConfig) -> Vec<crate::config::ResolvedSkillEntry> {
    effective_mcp_with(profile).resolved_skills()
}

/// Build the per-instance skills registry from the patched profile.
fn build_skills_registry_with(profile: &ProfileConfig) -> Arc<crate::skills::SkillsRegistry> {
    let entries = effective_skills_with(profile);
    let registry = Arc::new(crate::skills::SkillsRegistry::new(entries));
    if let Err(err) = registry.reload() {
        tracing::warn!(%err, "acp::adapter: per-instance skills initial reload failed");
    }
    registry
}

/// Single source of truth for the captain-intended `ProfileConfig`
/// at spawn time. Every consumer that needs to ask "what does the
/// captain want?" — `ResolvedInstance` builder, MCP registry
/// builder, skills registry builder, session-info shape — calls
/// this and reads from the returned profile.
///
/// Resolution order:
///   1. Pick base profile via `base_profile_for_patches` (errors
///      when neither `--profile <id>` nor `[profile] default`
///      addresses a real `[[profiles]]` entry).
///   2. Fold root `[[patches]]` from the captain's on-disk config,
///      filtered by each patch's optional `$match.profile` glob.
///   3. Fold `external_patches` in declaration order (the
///      `--with-config` per-invocation overrides). Empty slice
///      is a no-op.
///   4. Deserialize back to `ProfileConfig` + re-run garde
///      validation against the post-merge shape.
///
/// Before the hoist, layers drifted: `ResolvedInstance::from_config`
/// applied root patches to system_prompt / model / mode, but the
/// downstream `build_mcp_registry_with` / `build_skills_registry_with`
/// got the unpatched profile via `profile_by_id_in` — so patches'
/// `mcps` / `mcp.skills` silently never reached the spawned actor.
pub(crate) fn resolve_effective_profile(
    cfg: &Config,
    profile_id: Option<&str>,
    external_patches: &[Value],
) -> Result<ProfileConfig, RpcError> {
    let base = base_profile_for_patches(cfg, profile_id)?;
    let base_value =
        serde_json::to_value(&base).map_err(|e| RpcError::internal_error(format!("profile serialize failed: {e}")))?;

    let with_root = match cfg.patches.as_deref() {
        Some(rp) if !rp.is_empty() => crate::config::patch::apply_root_patches_to_profile(base_value, rp, &base.id),
        _ => base_value,
    };

    let merged = if external_patches.is_empty() {
        with_root
    } else {
        crate::config::patch::merge_patches(with_root, external_patches.to_vec())
    };

    let patched: ProfileConfig = serde_json::from_value(merged)
        .map_err(|e| RpcError::invalid_params(format!("profile resolution: invalid shape after patches: {e}")))?;
    garde::Validate::validate(&patched)
        .map_err(|e| RpcError::invalid_params(format!("profile resolution: validation failed: {e}")))?;
    Ok(patched)
}

/// Pick the base `ProfileConfig` patches will fold onto. Resolves
/// `--profile <id>` first, then `[profile] default`. Errors when
/// neither addresses a real `[[profiles]]` entry — every spawn
/// flows through a profile (no bare-agent fallback). Validation at
/// config-load already rejects an empty `[[profiles]]` list, so the
/// captain's setup mistake surfaces at daemon boot rather than per
/// `--with-config` invocation.
fn base_profile_for_patches(cfg: &Config, profile_id: Option<&str>) -> Result<ProfileConfig, RpcError> {
    if let Some(id) = profile_id {
        if let Some(p) = cfg.profiles.iter().find(|p| p.id == id) {
            return Ok(p.clone());
        }
        return Err(RpcError::invalid_params(format!(
            "profile '{id}' not found in [[profiles]] registry"
        )));
    }
    if let Some(default_id) = cfg.profile.default.as_deref() {
        if let Some(p) = cfg.profiles.iter().find(|p| p.id == default_id) {
            return Ok(p.clone());
        }
        return Err(RpcError::invalid_params(format!(
            "[profile] default = '{default_id}' but no matching [[profiles]] entry exists"
        )));
    }
    Err(RpcError::invalid_params(
        "no profile addressed and no `[profile] default` configured — every spawn requires a `[[profiles]]` entry. \
         Pass `--profile <id>` or set `[profile] default = '<id>'`.",
    ))
}

/// One-stop spawn-time resolver: pick + patch the profile, project
/// onto a `ResolvedInstance`, return both. The patched
/// `ProfileConfig` is the single source the MCP registry, skills
/// registry, and per-instance context downstream all read from.
///
/// `external_patches` is empty for plain submit / spawn paths; the
/// `--with-config` and `withConfig` RPC paths supply non-empty
/// patches that fold on top of root `[[patches]]`.
///
/// Explicit `agent_id` wins over whatever agent the patched profile
/// names — captain intent for "run THIS profile but on a different
/// vendor binary".
fn resolve_into_instance_and_profile(
    cfg: &Config,
    agent_id: Option<&str>,
    profile_id: Option<&str>,
    external_patches: &[Value],
) -> Result<(ResolvedInstance, ProfileConfig), RpcError> {
    let patched = resolve_effective_profile(cfg, profile_id, external_patches)?;
    let mut resolved = ResolvedInstance::from_profile_explicit(&patched, cfg)
        .map_err(|e| RpcError::invalid_params(format!("{e:#}")))?;

    if let Some(wanted) = agent_id {
        let agent = cfg
            .agents
            .agents
            .iter()
            .find(|a| a.id == wanted)
            .cloned()
            .ok_or_else(|| RpcError::invalid_params(format!("agent '{wanted}' not found in [[agents]] registry")))?;
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

    Ok((resolved, patched))
}

/// User-facing "agent exited before accepting our prompt" message.
/// Reads the rolling stderr tail off the handle so the captain sees
/// WHY the agent died (typical: bunx cache miss, missing OAuth token,
/// model name rejected) without having to grep the daemon log.
fn prompt_actor_closed_message(handle: &AcpInstance) -> String {
    let tail = handle.recent_stderr();
    if tail.is_empty() {
        "instance actor closed before accepting prompt (no stderr captured — agent died silently)".into()
    } else {
        format!(
            "instance actor closed before accepting prompt — agent stderr (last {} lines): {}",
            tail.len(),
            tail.join(" / ")
        )
    }
}

/// Same shape for the `list_sessions` ephemeral-actor path. The
/// ephemeral handle is owned locally (never registered) — we read its
/// stderr tail directly.
fn list_actor_closed_summary(handle: &AcpInstance) -> String {
    let tail = handle.recent_stderr();
    if tail.is_empty() {
        "no stderr captured".into()
    } else {
        format!("agent stderr (last {}): {}", tail.len(), tail.join(" / "))
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
        GenEvt::SelectedProfileChanged { .. } => "acp:profile-changed",
        GenEvt::SessionInfoUpdate { .. } => "acp:session-info-update",
        GenEvt::CurrentModeUpdate { .. } => "acp:current-mode-update",
        GenEvt::UsageUpdate { .. } => "acp:usage-update",
        GenEvt::ConfigOptionsUpdate { .. } => "acp:config-options-update",
        GenEvt::InstanceMeta { .. } => "acp:instance-meta",
        GenEvt::SystemPromptInjected { .. } => "acp:system-prompt-injected",
        GenEvt::QueueChanged { .. } => "acp:queue-changed",
        GenEvt::NotificationsChanged { .. } => "acp:notifications-changed",
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

        let (resolved, _) = adapter.resolve(None, Some("strict")).expect("strict resolves");
        assert_eq!(resolved.agent.id, "claude-code");
        assert_eq!(resolved.profile_id.as_deref(), Some("strict"));
        assert_eq!(resolved.model.as_deref(), Some("claude-opus-4-5"));
        assert_eq!(
            resolved.system_prompt_for(&Bootstrap::Fresh).as_deref(),
            Some("be terse")
        );

        let (resolved, _) = adapter.resolve(None, None).expect("default profile resolves");
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
[profile]
default = "personal"

[[agents]]
id = "claude-code"
provider = "acp-claude-code"
command = "/bin/false"

[[profiles]]
id = "personal"
agent = "claude-code"

[profiles.mcp]
skills = [{{ dir = "{dir}", ignore = ["work-*"] }}]

[[profiles]]
id = "work"
agent = "claude-code"

[profiles.mcp]
skills = [{{ dir = "{dir}", ignore = ["personal-*"] }}]

[[profiles]]
id = "no-skills"
agent = "claude-code"

[profiles.mcp]
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

        let personal_reg = adapter.build_skills_registry_for(personal.as_ref().expect("personal profile present"));
        let personal_slugs: Vec<String> = personal_reg.list().iter().map(|s| s.slug.to_string()).collect();
        assert!(personal_slugs.contains(&"personal-todo".into()));
        assert!(personal_slugs.contains(&"shared-readme".into()));
        assert!(
            !personal_slugs.contains(&"work-internal".into()),
            "personal profile must filter out work-* per its own glob"
        );

        let work_reg = adapter.build_skills_registry_for(work.as_ref().expect("work profile present"));
        let work_slugs: Vec<String> = work_reg.list().iter().map(|s| s.slug.to_string()).collect();
        assert!(work_slugs.contains(&"work-internal".into()));
        assert!(work_slugs.contains(&"shared-readme".into()));
        assert!(
            !work_slugs.contains(&"personal-todo".into()),
            "work profile must filter out personal-* per its own glob (NOT inherit personal's filter)"
        );

        let empty_reg = adapter.build_skills_registry_for(no_skills.as_ref().expect("no-skills profile present"));
        assert_eq!(
            empty_reg.list().len(),
            0,
            "skills = [] explicit off-switch yields empty registry"
        );
    }

    /// Pins the reviewer's blocking finding pre-hoist: root
    /// `[[patches]]` `mcps` / `mcp.skills` reach the MCP / skills
    /// registry the spawned actor sees. Before `resolve_effective_profile`
    /// was hoisted, the empty-`--with-config` path returned the
    /// UNPATCHED profile to `build_mcp_registry_with` /
    /// `build_skills_registry_with`, silently dropping the captain's
    /// patch-supplied content. Test asserts both paths see the patch.
    #[tokio::test]
    async fn root_patches_reach_mcp_and_skills_registries_on_default_spawn() {
        let skills_dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(skills_dir.path().join("from-patch")).unwrap();
        std::fs::write(
            skills_dir.path().join("from-patch/SKILL.md"),
            "---\nname: from-patch\ndescription: pinned by root patch\n---\nbody\n",
        )
        .unwrap();
        let mcps_path = skills_dir.path().join("mcps.json");
        std::fs::write(
            &mcps_path,
            r#"{"mcpServers": {"from-patch": {"command": "/bin/false"}}}"#,
        )
        .unwrap();

        let cfg: Config = toml::from_str(&format!(
            r#"
[[agents]]
id = "cc"
provider = "acp-claude-code"
command = "/bin/false"

[profile]
default = "p1"

[[profiles]]
id = "p1"
agent = "cc"

[[patches]]
[[patches.mcps]]
file = "{mcps}"

[patches.mcp]
enabled = true
[[patches.mcp.skills]]
dir = "{skills}"
"#,
            mcps = mcps_path.display(),
            skills = skills_dir.path().display(),
        ))
        .expect("fixture parses");
        cfg.validate().expect("fixture validates");

        let adapter = AcpAdapter::new(cfg, Arc::new(StatusBroadcast::new(true)));
        let (_resolved, patched) = adapter
            .resolve(None, None)
            .expect("default profile resolves with patches applied");

        // Patches landed on the profile.
        assert!(
            patched.mcps.as_deref().is_some_and(|v| !v.is_empty()),
            "patches' mcps must reach the resolved profile"
        );
        assert!(
            patched.mcp.as_ref().is_some_and(|m| m.skills.is_some()),
            "patches' mcp.skills must reach the resolved profile"
        );

        // The MCP registry builds against the patched profile.
        let skills = build_skills_registry_with(&patched);
        let mcp_registry = build_mcp_registry_with(&patched, Some(&skills));
        assert!(
            mcp_registry.is_some(),
            "patched mcps must produce a non-empty MCPsRegistry"
        );
        let slugs: Vec<String> = skills.list().iter().map(|s| s.slug.to_string()).collect();
        assert!(
            slugs.contains(&"from-patch".to_string()),
            "patch-supplied skill `from-patch` must reach the skills registry, got: {slugs:?}"
        );
    }

    #[test]
    fn mcp_config_globs_apply_to_external_servers_without_per_server_policy() {
        let mut defs = vec![crate::mcp::MCPDefinition {
            name: "memory".into(),
            raw: serde_json::json!({ "command": "/bin/false" }),
            hyprpilot: crate::mcp::HyprpilotExtension::default(),
            source: std::path::PathBuf::from("test.json"),
        }];
        let cfg = crate::config::McpConfig {
            auto_accept_tools: Some(vec!["read_*".into()]),
            auto_reject_tools: Some(vec!["delete_*".into()]),
            ..Default::default()
        };

        apply_mcp_glob_defaults(&mut defs, &cfg);

        assert_eq!(defs[0].hyprpilot.auto_accept_tools, vec!["read_*"]);
        assert_eq!(defs[0].hyprpilot.auto_reject_tools, vec!["delete_*"]);
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
[[agents]]
id = "dead"
provider = "acp-claude-code"
command = "/bin/false"

[profile]
default = "dead"

[[profiles]]
id = "dead"
agent = "dead"
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

    /// `withConfig` happy path — patches fold against the resolved
    /// profile so a patch can override `model` / `mode` / `agent`
    /// from the captain's invocation. Here the spawn picks the
    /// `[profile] default = "base"` profile, then the patch swaps
    /// `agent` to "extra" (which exists in the registry) and sets
    /// `model`.
    #[tokio::test]
    async fn spawn_with_config_patch_overrides_resolved_profile() {
        let cfg: Config = toml::from_str(
            r#"
[[agents]]
id = "base"
provider = "acp-claude-code"
command = "/bin/false"

[[agents]]
id = "extra"
provider = "acp-claude-code"
command = "/bin/false"

[profile]
default = "base"

[[profiles]]
id = "base"
agent = "base"
"#,
        )
        .expect("parses");
        let adapter = AcpAdapter::new(cfg, Arc::new(StatusBroadcast::new(true)));

        // Profile-shaped patch: target the resolved profile directly,
        // swapping agent + overriding model. Patch fields mirror
        // `[[profiles]]` TOML shape.
        let patch = serde_json::json!({
            "agent": "extra",
            "model": "claude-opus-4-7"
        });

        let spec = SpawnSpec {
            config_patches: vec![patch],
            ..Default::default()
        };
        let key = adapter
            .spawn_instance(spec)
            .await
            .expect("spawn ok with patched profile");
        let info = <AcpAdapter as Adapter>::info_for(&adapter, key)
            .await
            .expect("info_for");
        assert_eq!(info.agent_id, "extra");
    }

    /// `--with-config` validation path — a patch that produces an
    /// invalid config (unknown field surfaces via `deny_unknown_fields`)
    /// must error before the spawn completes; no instance leaks into
    /// the registry.
    #[tokio::test]
    async fn spawn_with_config_patch_rejects_unknown_field() {
        let cfg: Config = toml::from_str(
            r#"
[[agents]]
id = "base"
provider = "acp-claude-code"
command = "/bin/false"

[profile]
default = "base"

[[profiles]]
id = "base"
agent = "base"
"#,
        )
        .expect("parses");
        let adapter = AcpAdapter::new(cfg, Arc::new(StatusBroadcast::new(true)));

        let patch = serde_json::json!({
            "this_field_does_not_exist": true
        });

        let spec = SpawnSpec {
            config_patches: vec![patch],
            ..Default::default()
        };
        let err = adapter.spawn_instance(spec).await.expect_err("typo must reject");
        assert_eq!(err.code, -32602);
        assert!(
            err.message.contains("profile resolution") || err.message.contains("unknown field"),
            "expected a captain-facing profile-resolution error, got: {}",
            err.message
        );
    }
}
