//! Tauri `#[command]`s live at the generic adapter layer (not under
//! `acp/`). Commands that need `dyn Adapter` call through the trait;
//! commands that need config-adjacent surfaces (`agents_list`,
//! `profiles_list`, `session_load`) pull the concrete `AcpAdapter`
//! from managed state. Adding an HTTP sibling later splits those
//! config-adjacent commands per-adapter or hoists the concept to
//! trait level.

use std::path::PathBuf;
use std::sync::Arc;

use agent_client_protocol::schema::ListSessionsResponse;
use serde::Serialize;
use serde_json::{json, Value};
use tauri::State;

use super::acp::AcpAdapter;
use super::instance::InstanceKey;
use super::mirror::{ChatSnapshot, MetaSnapshot, TerminalsSnapshot};
use super::permission::PermissionController;
use super::transcript::Attachment;
use super::Adapter;
use crate::completion::hydration::TokenHydrators;

type AdapterState<'a> = State<'a, Arc<AcpAdapter>>;
type HydratorsState<'a> = State<'a, TokenHydrators>;

#[tauri::command]
pub async fn session_submit(
    adapter: AdapterState<'_>,
    hydrators: HydratorsState<'_>,
    text: String,
    attachments: Option<Vec<Attachment>>,
    instance_id: Option<String>,
    agent_id: Option<String>,
    profile_id: Option<String>,
) -> Result<Value, String> {
    let mut attachments = attachments.unwrap_or_default();
    // Hydrate inline `#{<scheme>://<value>}` tokens via the generic
    // hydrator registry. Today only `skills://` is registered; the
    // dispatcher walks every token, finds the matching scheme owner,
    // and projects the value into an `Attachment`. Unknown
    // schemes / unresolved values warn-and-drop. Token text stays
    // visible in the chat so the captain sees what they referenced.
    let hydrated = hydrators.hydrate_all(&text).await;
    if !hydrated.is_empty() {
        tracing::debug!(count = hydrated.len(), "cmd::session_submit: hydrated tokens");
        attachments.extend(hydrated);
    }
    tracing::info!(
        text_len = text.len(),
        attachments = attachments.len(),
        instance_id = ?instance_id,
        agent_id = ?agent_id,
        profile_id = ?profile_id,
        "cmd::session_submit: entry"
    );
    let out = adapter
        .submit_prompt(
            &text,
            &attachments,
            instance_id.as_deref(),
            agent_id.as_deref(),
            profile_id.as_deref(),
        )
        .await
        .map_err(|e| e.message);
    match &out {
        Ok(_) => tracing::info!("cmd::session_submit: accepted"),
        Err(err) => tracing::warn!(%err, "cmd::session_submit: failed"),
    }
    out
}

#[tauri::command]
pub async fn session_cancel(
    adapter: AdapterState<'_>,
    instance_id: Option<String>,
    agent_id: Option<String>,
) -> Result<Value, String> {
    tracing::info!(instance_id = ?instance_id, agent_id = ?agent_id, "cmd::session_cancel: entry");
    let out = adapter
        .cancel_active(instance_id.as_deref(), agent_id.as_deref())
        .await
        .map_err(|e| e.message);
    if let Err(err) = &out {
        tracing::warn!(%err, "cmd::session_cancel: failed");
    }
    out
}

/// Mirror of the `instances/restart` JSON-RPC method for the webview.
/// `cwd` is optional — supplying it overlays the resolved agent cwd
/// before the post-restart actor spawns. Drives the cwd palette.
///
/// `ensure: true` mirrors `instance_meta`'s flag — when no live actor
/// matches `instance_id` (or none is supplied), the daemon resolves
/// `(agent_id, profile_id)` and spawns a fresh instance rooted at
/// `cwd` instead of erroring. Lets the cwd palette work on a
/// fresh-boot daemon with empty registry.
#[tauri::command]
pub async fn instance_restart(
    adapter: AdapterState<'_>,
    instance_id: Option<String>,
    cwd: Option<PathBuf>,
    ensure: Option<bool>,
    agent_id: Option<String>,
    profile_id: Option<String>,
) -> Result<Value, String> {
    let ensure = ensure.unwrap_or(false);
    tracing::info!(
        instance_id = ?instance_id,
        cwd = ?cwd,
        ensure,
        agent_id = ?agent_id,
        profile_id = ?profile_id,
        "cmd::instance_restart: entry"
    );
    let key = match instance_id.as_deref() {
        Some(id) => Some(InstanceKey::parse(id).map_err(|e| e.to_string())?),
        None => None,
    };
    let out = adapter
        .restart_instance(key, cwd, ensure, agent_id.as_deref(), profile_id.as_deref())
        .await
        .map_err(|e| e.message);
    match &out {
        Ok(_) => tracing::info!("cmd::instance_restart: accepted"),
        Err(err) => tracing::warn!(%err, "cmd::instance_restart: failed"),
    }
    out.map(|key| serde_json::json!({ "instanceId": key.as_string() }))
}

/// `agents_list` / `profiles_list` ride a one-field envelope so the
/// UI's `TauriCommandResult[AgentsList]` shape line up with the
/// `{ agents: [...] }` / `{ profiles: [...] }` JSON the daemon
/// emits. Typed structs over hand-rolled `json!()` keep the wire
/// shape lock-stepped to the typed list members.
#[derive(Debug, serde::Serialize)]
pub struct AgentsListEnvelope {
    pub agents: Vec<crate::adapters::AgentSummary>,
}

#[derive(Debug, serde::Serialize)]
pub struct ProfilesListEnvelope {
    pub profiles: Vec<crate::adapters::ProfileSummary>,
}

#[tauri::command]
pub async fn agents_list(adapter: AdapterState<'_>) -> Result<AgentsListEnvelope, String> {
    Ok(AgentsListEnvelope {
        agents: adapter.list_agents(),
    })
}

#[tauri::command]
pub async fn profiles_list(adapter: AdapterState<'_>) -> Result<ProfilesListEnvelope, String> {
    Ok(ProfilesListEnvelope {
        profiles: adapter.list_profiles(),
    })
}

#[tauri::command]
pub async fn session_list(
    adapter: AdapterState<'_>,
    instance_id: Option<String>,
    agent_id: Option<String>,
    profile_id: Option<String>,
    cwd: Option<PathBuf>,
) -> Result<ListSessionsResponse, String> {
    tracing::info!(
        instance_id = ?instance_id,
        agent_id = ?agent_id,
        profile_id = ?profile_id,
        cwd = ?cwd,
        "cmd::session_list: entry"
    );
    let out = adapter
        .list_sessions(instance_id.as_deref(), agent_id.as_deref(), profile_id.as_deref(), cwd)
        .await
        .map_err(|e| e.message);
    if let Err(err) = &out {
        tracing::warn!(%err, "cmd::session_list: failed");
    }
    out
}

/// Single-session projection returned by `sessions_info`. Mirrors the
/// `sessions/info` RPC handler — one session by id with the resolved
/// agent/profile riding back so the palette preview can correlate the
/// row to a known profile.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionInfoResult {
    pub id: String,
    pub title: Option<String>,
    pub cwd: String,
    pub last_turn_at: Option<String>,
    pub message_count: Option<u64>,
    pub agent_id: String,
    pub profile_id: Option<String>,
}

#[tauri::command]
pub async fn sessions_info(adapter: AdapterState<'_>, id: String) -> Result<SessionInfoResult, String> {
    tracing::info!(session_id = %id, "cmd::sessions_info: entry");
    // No ACP `session/get` verb — list + filter, mirroring the
    // `sessions/info` RPC handler. Default agent/profile resolution.
    let resp = adapter
        .list_sessions(None, None, None, None)
        .await
        .map_err(|e| e.message)?;
    let info = resp
        .sessions
        .iter()
        .find(|s| s.session_id.0.as_ref() == id.as_str())
        .ok_or_else(|| format!("no session '{id}'"))?;
    let (agent_id, profile_id) = {
        let cfg = adapter.config.read().expect("AcpAdapter config lock poisoned");
        // Every spawn flows through a profile; for the session-info
        // shape we pick `[profile] default`, falling back to the first
        // entry, then read its agent. Config-load validation ensures
        // at least one profile exists.
        let profile = cfg
            .profile
            .default
            .as_deref()
            .and_then(|id| cfg.profiles.iter().find(|p| p.id == id))
            .or_else(|| cfg.profiles.first());
        let agent_id = profile.map(|p| p.agent.clone()).unwrap_or_default();
        let profile_id = profile.map(|p| p.id.clone());
        (agent_id, profile_id)
    };
    Ok(SessionInfoResult {
        id: info.session_id.0.to_string(),
        title: info.title.clone(),
        cwd: info.cwd.display().to_string(),
        last_turn_at: info.updated_at.clone(),
        message_count: None,
        agent_id,
        profile_id,
    })
}

#[tauri::command]
pub async fn session_load(
    adapter: AdapterState<'_>,
    instance_id: Option<String>,
    agent_id: Option<String>,
    profile_id: Option<String>,
    session_id: String,
    cwd: Option<PathBuf>,
    with_config: Option<Vec<Value>>,
) -> Result<(), String> {
    let with_config = with_config.unwrap_or_default();
    tracing::info!(
        instance_id = ?instance_id,
        agent_id = ?agent_id,
        profile_id = ?profile_id,
        session_id = %session_id,
        cwd = ?cwd,
        with_config_count = with_config.len(),
        "cmd::session_load: entry"
    );
    let out = adapter
        .load_session(
            instance_id.as_deref(),
            agent_id.as_deref(),
            profile_id.as_deref(),
            session_id,
            cwd,
            with_config,
        )
        .await
        .map_err(|e| e.message);
    match &out {
        Ok(_) => tracing::info!("cmd::session_load: accepted"),
        Err(err) => tracing::warn!(%err, "cmd::session_load: failed"),
    }
    out.map(|_| ())
}

/// List every live instance the adapter knows about. Mirrors the
/// `instances/list` JSON-RPC method; used by the instances palette
/// leaf to drive its row list. Returns the same shape the JSON-RPC
/// handler emits so UI code reading either surface treats them
/// uniformly.
///
/// `focusedId` ships alongside the list so a remote bridge that just
/// authenticated mid-session knows which instance the daemon is
/// currently focused on — without it, the remote's `useActiveInstance`
/// stays empty until the next focus-event fires (which on a
/// long-idle desktop may be never). UI brim-sync reads the field and
/// calls `applyFocus(focusedId, 'manual')` to seed the active-instance
/// pointer.
#[tauri::command]
pub async fn instances_list(adapter: AdapterState<'_>) -> Result<Value, String> {
    let items = adapter.list().await;
    let focused_id = adapter.focused_id().await.map(|k| k.as_string());
    let wire: Vec<crate::adapters::InstanceListEntry> =
        items.iter().map(crate::adapters::InstanceListEntry::from).collect();
    // Omit `focusedId` from the JSON when no instance is focused —
    // serializing `Option<String>` as `null` lets a typo-prone UI
    // path coerce null into a `setIfUnset(null)` call, breaking the
    // active-instance pointer. Skipping the key entirely makes the
    // UI see `undefined`, which the `r.focusedId === undefined`
    // guards correctly handle.
    let mut payload = serde_json::Map::with_capacity(2);
    payload.insert(
        "instances".into(),
        serde_json::to_value(&wire).map_err(|e| format!("serialize instances: {e}"))?,
    );
    if let Some(id) = focused_id {
        payload.insert("focusedId".into(), Value::String(id));
    }
    Ok(Value::Object(payload))
}

#[tauri::command]
pub async fn instances_focus(adapter: AdapterState<'_>, instance_id: String) -> Result<Value, String> {
    let key = InstanceKey::parse(&instance_id).map_err(|e| e.to_string())?;
    let key = adapter.focus(key).await.map_err(|e| e.to_string())?;
    Ok(json!({ "instanceId": key.as_string() }))
}

#[tauri::command]
pub async fn instances_shutdown(adapter: AdapterState<'_>, instance_id: String) -> Result<Value, String> {
    let key = InstanceKey::parse(&instance_id).map_err(|e| e.to_string())?;
    let key = adapter.shutdown_one(key).await.map_err(|e| e.to_string())?;
    Ok(json!({ "instanceId": key.as_string() }))
}

/// Rename a live instance. `instanceId` accepts UUID or current
/// name; `name` is `None` (clear) or a slug-validated string. The
/// actual slug validation runs inside `Adapter::rename` so the wire
/// shape stays consistent with the RPC handler.
#[tauri::command]
pub async fn instances_rename(
    adapter: AdapterState<'_>,
    instance_id: String,
    name: Option<String>,
) -> Result<Value, String> {
    let key = adapter
        .resolve_token(&instance_id)
        .await
        .ok_or_else(|| format!("instance '{instance_id}' not found"))?;
    adapter.rename(key, name.clone()).await.map_err(|e| e.to_string())?;
    Ok(json!({
        "instanceId": key.as_string(),
        "name": name,
    }))
}

/// Switch the active model for the addressed instance. Today returns
/// the same `-32603`-shaped error the `models/set` wire handler does —
/// `AcpAdapter::set_session_model` stubs past the membership check
/// until the runtime side is wired. The UI surfaces the message via
/// toast.
#[tauri::command]
pub async fn models_set(adapter: AdapterState<'_>, instance_id: String, model_id: String) -> Result<Value, String> {
    tracing::info!(instance_id = %instance_id, model_id = %model_id, "cmd::models_set: entry");
    let out = adapter
        .set_session_model(&instance_id, &model_id)
        .await
        .map_err(|e| e.message);
    if let Err(err) = &out {
        tracing::warn!(%err, "cmd::models_set: failed");
    }
    out
}

/// Switch the active operational mode for the addressed instance.
/// Mirrors `models_set` — stubbed at the adapter until the runtime
/// side is wired.
#[tauri::command]
pub async fn modes_set(adapter: AdapterState<'_>, instance_id: String, mode_id: String) -> Result<Value, String> {
    tracing::info!(instance_id = %instance_id, mode_id = %mode_id, "cmd::modes_set: entry");
    let out = adapter
        .set_session_mode(&instance_id, &mode_id)
        .await
        .map_err(|e| e.message);
    if let Err(err) = &out {
        tracing::warn!(%err, "cmd::modes_set: failed");
    }
    out
}

/// Set a generic ACP `session/set_config_option`. Sibling to
/// `modes_set` / `models_set` — those address the dedicated wire
/// methods; this one is the catch-all the captain uses to flip any
/// other config knob the agent advertises (e.g. claude-code's
/// `thought_level`, codex's policy / `_*` extension categories,
/// vendor-specific selectors).
///
/// **Usage:** the agent emits `configOptions: [{ id, name,
/// currentValue, kind: { type: "select", options: [...] } }]` on
/// `NewSessionResponse` (and refreshes the same shape via
/// `config_option_update` notifications). The palette surfaces every
/// advertised option, captain picks one, this command sends the new
/// value. The agent's response carries the full updated configOptions
/// array — captain's pick may have side effects on other options
/// (e.g. switching `model` may change `thought_level`'s available
/// values).
///
/// Spec note: option ids beginning with `_` are vendor-extension
/// freeform; ids without `_` are reserved spec categories
/// (`mode` / `model` / `thought_level`). Vendors that surface a
/// reserved category here AND on the dedicated wire method
/// (`set_mode` / `set_model`) — the captain picks one path; both
/// trigger the agent's same internal config path.
#[tauri::command]
pub async fn config_option_set(
    adapter: AdapterState<'_>,
    instance_id: String,
    config_id: String,
    value: String,
) -> Result<Value, String> {
    tracing::info!(
        instance_id = %instance_id,
        config_id = %config_id,
        value = %value,
        "cmd::config_option_set: entry"
    );
    let out = adapter
        .set_session_config_option(&instance_id, &config_id, &value)
        .await
        .map_err(|e| e.message);
    if let Err(err) = &out {
        tracing::warn!(%err, "cmd::config_option_set: failed");
    }
    out
}

/// Read the daemon's currently-selected default profile id.
/// `Option<String>` so a fresh boot before any profile is configured
/// renders as `null` on the wire.
#[tauri::command]
pub async fn profile_get(adapter: AdapterState<'_>) -> Result<Option<String>, String> {
    Ok(adapter.selected_profile_id())
}

/// Mutate the daemon's currently-selected default profile. Publishes
/// `acp:profile-changed` so every frontend (Vue overlay, nvim plugin,
/// ctl) syncs its header pill + palette active marker without polling.
#[tauri::command]
pub async fn profile_set(adapter: AdapterState<'_>, profile_id: String) -> Result<Value, String> {
    tracing::info!(profile_id = %profile_id, "cmd::profile_set: entry");
    let out = adapter.set_selected_profile_id(&profile_id).map_err(|e| e.message);
    if let Err(err) = &out {
        tracing::warn!(%err, "cmd::profile_set: failed");
    }
    out
}

/// Snapshot the addressed instance's per-instance metadata
/// (cwd, advertised modes/models, current ids). The palette pickers
/// call this on every open instead of reading the UI-side
/// `useSessionInfo` cache — the daemon's per-instance Arc<RwLock>
/// is the authoritative source, refreshed on every session/new,
/// session/load, set_mode, set_model, and turn-end. UI events
/// (`acp:instance-meta`) keep the cache mirror in sync; this
/// command exists for the "always re-ask the daemon" idiom the
/// pickers want regardless.
#[tauri::command]
pub async fn instance_meta(
    adapter: AdapterState<'_>,
    instance_id: Option<String>,
    ensure: Option<bool>,
    agent_id: Option<String>,
    profile_id: Option<String>,
) -> Result<Value, String> {
    let ensure = ensure.unwrap_or(false);
    tracing::debug!(instance_id = ?instance_id, ensure, agent_id = ?agent_id, profile_id = ?profile_id, "cmd::instance_meta: entry");

    if ensure {
        return adapter
            .instance_meta_or_ensure(instance_id.as_deref(), agent_id.as_deref(), profile_id.as_deref())
            .await
            .map_err(|e| e.message);
    }

    let id = instance_id.ok_or_else(|| "instance_id required when ensure=false".to_string())?;
    adapter.instance_meta(&id).await.map_err(|e| e.message)
}

/// Resolve a pending permission prompt with the captain's pick.
///
/// `option_id` MUST be one of the agent-offered option ids. Hyprpilot
/// is transparent to the agent's permission semantics — the captain
/// picks one of the offered options and we forward it verbatim;
/// "always" persistence is the agent's concern.
///
/// No-op when no waiter matches `request_id` (already resolved, timed
/// out, or never registered). The command never errors on that path —
/// the UI sees `Ok(())` regardless so a stale reply doesn't surface
/// as a user-visible failure.
///
/// `feedback` mirrors `permissions/respond { feedback }`: when the
/// captain rejects with a non-empty reason, the daemon dispatches a
/// synthetic follow-up `session/prompt` carrying the feedback as
/// user text so the agent sees the rejection's "why" on its next
/// turn. Ignored on allow-shaped picks and on empty / whitespace-
/// only strings.
#[tauri::command]
pub async fn permission_reply(
    adapter: AdapterState<'_>,
    controller: State<'_, Arc<dyn PermissionController>>,
    _session_id: String,
    request_id: String,
    option_id: String,
    feedback: Option<String>,
) -> Result<(), String> {
    tracing::info!(
        request_id = %request_id,
        option_id = %option_id,
        has_feedback = feedback.as_deref().is_some_and(|s| !s.trim().is_empty()),
        "cmd::permission_reply: entry"
    );
    let snapshot = controller.snapshot_for(&request_id).await;
    match controller.resolve_if_pending(&request_id, &option_id).await {
        None => {
            tracing::debug!(request_id, "permission_reply: no waiter — no-op");
            return Ok(());
        }
        Some(false) => {
            tracing::warn!(
                request_id,
                option_id,
                "permission_reply: option_id not in offered set — no-op"
            );
            return Ok(());
        }
        Some(true) => {
            tracing::info!(request_id, option_id, "cmd::permission_reply: resolved");
        }
    }

    // Feedback-on-reject follow-up — mirrors the `permissions/respond`
    // handler's behaviour so desktop SPA + external JSON-RPC clients
    // share one semantic path. Detached: command returns immediately;
    // the follow-up turn races toward the actor's mpsc and lands as
    // a normal `session/prompt`.
    if let Some(text) = feedback.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let Some(snap) = snapshot else { return Ok(()) };
        let picked_kind = snap
            .options
            .iter()
            .find(|o| o.option_id == option_id)
            .map(|o| o.kind.clone());
        let is_reject = picked_kind
            .as_deref()
            .is_some_and(crate::adapters::permission::is_reject_kind);
        if !is_reject {
            return Ok(());
        }
        let Some(instance_id) = snap.instance_id else {
            tracing::warn!(
                request_id = %request_id,
                "cmd::permission_reply: reject feedback supplied but no instance id — dropped"
            );
            return Ok(());
        };
        let adapter_arc = adapter.inner().clone();
        let text_owned = text.to_string();
        tokio::spawn(async move {
            let adapter_dyn: std::sync::Arc<dyn crate::adapters::Adapter> = adapter_arc;
            match adapter_dyn
                .submit(
                    crate::adapters::UserTurnInput::Prompt {
                        text: text_owned,
                        attachments: Vec::new(),
                    },
                    Some(instance_id.as_str()),
                    None,
                    None,
                )
                .await
            {
                Ok(_) => {
                    tracing::info!(
                        instance_id = %instance_id,
                        "cmd::permission_reply: reject feedback dispatched as follow-up turn"
                    );
                }
                Err(err) => {
                    tracing::warn!(
                        instance_id = %instance_id,
                        error = ?err,
                        "cmd::permission_reply: feedback follow-up failed"
                    );
                }
            }
        });
    }
    Ok(())
}

/// Snapshot the addressed instance's `MetaSnapshot` off the
/// per-instance write-through mirror. UI reads this on focus-switch
/// for brim-sync hydration.
///
/// Coexists with `instance_meta` above: that command goes through the
/// actor's `MetaSnapshot` command (a roundtrip to the live actor's
/// command loop, surfacing the agent's cached `availableModels` /
/// `availableModes` even before the first event lands). This one
/// reads the mirror directly — same per-event state, no actor
/// roundtrip — and complements it for the post-event hydration path.
#[tauri::command]
pub async fn instance_snapshot_meta(adapter: AdapterState<'_>, instance_id: String) -> Result<MetaSnapshot, String> {
    let key = InstanceKey::parse(&instance_id).map_err(|e| e.to_string())?;
    let mirror = adapter
        .instance_mirror(key)
        .await
        .ok_or_else(|| format!("instance '{instance_id}' not found in registry"))?;
    let snap = mirror.meta_snapshot().await;
    tracing::trace!(
        target: "snapshot::meta",
        instance_id = %instance_id,
        turns = snap.turns.len(),
        pending_permissions = snap.pending_permissions.len(),
        latest_seq = ?snap.latest_seq,
        "served meta snapshot",
    );
    Ok(snap)
}

/// Snapshot a windowed page of the addressed instance's transcript.
/// `before` is a strictly-older cursor (chain `before = oldestSeq`
/// of the previous page to paginate backwards); `after` is a
/// strictly-newer cursor (delta-replay on remote reconnect). Unset
/// `before` + unset `after` → return the latest `limit` entries.
/// `limit = 0` or unset → mirror's default page size (50). `before`
/// and `after` are mutually exclusive — caller error if both set.
#[tauri::command]
pub async fn instance_snapshot_chat(
    adapter: AdapterState<'_>,
    instance_id: String,
    before: Option<u64>,
    after: Option<u64>,
    limit: Option<usize>,
) -> Result<ChatSnapshot, String> {
    if before.is_some() && after.is_some() {
        return Err("instance_snapshot_chat: `before` and `after` are mutually exclusive".to_string());
    }
    let key = InstanceKey::parse(&instance_id).map_err(|e| e.to_string())?;
    let mirror = adapter
        .instance_mirror(key)
        .await
        .ok_or_else(|| format!("instance '{instance_id}' not found in registry"))?;
    let snap = mirror.chat_snapshot(before, after, limit.unwrap_or(0)).await;
    tracing::trace!(
        target: "snapshot::chat",
        instance_id = %instance_id,
        before = ?before,
        after = ?after,
        items = snap.items.len(),
        oldest_seq = ?snap.oldest_seq,
        latest_seq = ?snap.latest_seq,
        has_more = snap.has_more,
        "served chat snapshot page",
    );
    Ok(snap)
}

/// Snapshot every per-`terminal_id` map entry. Small enough to ship
/// whole today; revisit if a session accumulates dozens of long-
/// running terminals.
#[tauri::command]
pub async fn instance_snapshot_terminals(
    adapter: AdapterState<'_>,
    instance_id: String,
) -> Result<TerminalsSnapshot, String> {
    let key = InstanceKey::parse(&instance_id).map_err(|e| e.to_string())?;
    let mirror = adapter
        .instance_mirror(key)
        .await
        .ok_or_else(|| format!("instance '{instance_id}' not found in registry"))?;
    let snap = mirror.terminals_snapshot().await;
    tracing::trace!(
        target: "snapshot::terminals",
        instance_id = %instance_id,
        terminals = snap.terminals.len(),
        "served terminals snapshot",
    );
    Ok(snap)
}

/// Read-only snapshot of the resolved MCP set for an instance. When
/// `instance_id` resolves to a live actor we resolve the catalog
/// through its profile (profile's `mcps` wholesale-replaces the
/// global default — same path as the ACP injection at session/new);
/// otherwise we fall back to the global set. Without the per-instance
/// resolution captains who scope their MCPs under `[[profiles]]
/// mcps = […]` see an empty palette while the live agent has the
/// Snapshot read of the per-instance queue from the daemon mirror.
/// Same shape as `instance_snapshot_meta` / `instance_snapshot_chat`
/// / `instance_snapshot_terminals` — no actor round-trip, just a
/// read off the cached `Vec<QueueItem>`. Powers the Vue UI's
/// first-observation hydration in `use-queue.ts::refreshQueue` so
/// the read path is symmetric with the other snapshot endpoints.
#[tauri::command]
pub async fn instance_snapshot_queue(adapter: AdapterState<'_>, instance_id: String) -> Result<Value, String> {
    use crate::adapters::Adapter;
    let key = InstanceKey::parse(&instance_id).map_err(|e| e.to_string())?;
    let mirror = adapter
        .instance_mirror(key)
        .await
        .ok_or_else(|| format!("instance '{instance_id}' not found in registry"))?;
    let items = mirror.queue_snapshot().await;

    Ok(serde_json::json!({ "items": items }))
}

/// `queue/list` Tauri mirror. Hydrates the captain-visible queue on
/// connect / focus / reconnect. `instance_id` falls back to the
/// focused instance when omitted; an empty queue returns `[]`, not
/// an error, so callers can render unconditionally.
#[tauri::command]
pub async fn queue_list(adapter: AdapterState<'_>, instance_id: Option<String>) -> Result<Value, String> {
    let items = adapter
        .queue_list(instance_id.as_deref())
        .await
        .map_err(|e| e.message)?;
    Ok(serde_json::json!({ "items": items }))
}

/// `queue/edit` Tauri mirror. In-place edit on a queued item;
/// preserves id + position + `enqueued_seq`. `attachments` left
/// unset (None on the wire) keeps the existing list; supplied
/// `Some([])` clears all attachments.
#[tauri::command]
pub async fn queue_edit(
    adapter: AdapterState<'_>,
    instance_id: Option<String>,
    item_id: String,
    text: String,
    attachments: Option<Vec<Attachment>>,
) -> Result<Value, String> {
    let item = adapter
        .queue_edit(instance_id.as_deref(), item_id, text, attachments)
        .await
        .map_err(|e| e.message)?;
    Ok(serde_json::json!({ "item": item }))
}

/// `queue/remove` Tauri mirror. Returns `{ removed: bool }` —
/// `false` when the id wasn't in the queue (already drained / never
/// existed) so the UI can no-op gracefully.
#[tauri::command]
pub async fn queue_remove(
    adapter: AdapterState<'_>,
    instance_id: Option<String>,
    item_id: String,
) -> Result<Value, String> {
    let removed = adapter
        .queue_remove(instance_id.as_deref(), item_id)
        .await
        .map_err(|e| e.message)?;
    Ok(serde_json::json!({ "removed": removed }))
}

/// `queue/move` Tauri mirror. Reorder; position clamps to
/// `[0, len-1]` server-side. Returns `{ moved: bool }`.
#[tauri::command]
pub async fn queue_move(
    adapter: AdapterState<'_>,
    instance_id: Option<String>,
    item_id: String,
    position: usize,
) -> Result<Value, String> {
    let moved = adapter
        .queue_move(instance_id.as_deref(), item_id, position)
        .await
        .map_err(|e| e.message)?;
    Ok(serde_json::json!({ "moved": moved }))
}

/// `queue/clear` Tauri mirror. Returns `{ cleared: u32 }` carrying
/// the count of dropped entries so the UI can render a toast.
#[tauri::command]
pub async fn queue_clear(adapter: AdapterState<'_>, instance_id: Option<String>) -> Result<Value, String> {
    let cleared = adapter
        .queue_clear(instance_id.as_deref())
        .await
        .map_err(|e| e.message)?;
    Ok(serde_json::json!({ "cleared": cleared }))
}

/// `queue/dispatch` Tauri mirror. Captain's "send now" — pops the
/// named item (or the head when omitted) AND dispatches it
/// immediately, bypassing the auto-queue. The returned payload
/// matches `QueueDispatchResult` from the adapter facade.
#[tauri::command]
pub async fn queue_dispatch(
    adapter: AdapterState<'_>,
    instance_id: Option<String>,
    item_id: Option<String>,
) -> Result<Value, String> {
    let res = adapter
        .queue_dispatch(instance_id.as_deref(), item_id)
        .await
        .map_err(|e| e.message)?;
    serde_json::to_value(res).map_err(|e| format!("queue_dispatch: serialise reply: {e}"))
}

/// servers wired in.
#[tauri::command]
pub async fn mcps_list(adapter: AdapterState<'_>, instance_id: Option<String>) -> Result<Value, String> {
    let catalog = adapter.resolve_mcp_catalog(instance_id.as_deref()).await;
    let items: Vec<Value> = catalog
        .iter()
        .map(|m| {
            serde_json::json!({
                "name": m.name,
                "raw": m.raw,
                "hyprpilot": {
                    "autoAcceptTools": m.hyprpilot.auto_accept_tools,
                    "autoRejectTools": m.hyprpilot.auto_reject_tools,
                },
                "source": crate::tools::path::display_cwd(&m.source.to_string_lossy()),
            })
        })
        .collect();
    Ok(serde_json::json!({ "mcps": items }))
}
