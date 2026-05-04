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
use super::permission::{PermissionController, TrustDecision};
use super::transcript::Attachment;
use super::Adapter;
use crate::completion::hydration::TokenHydrators;
use crate::mcp::MCPsRegistry;

type AdapterState<'a> = State<'a, Arc<AcpAdapter>>;
type MCPsState<'a> = State<'a, Arc<MCPsRegistry>>;
type HydratorsState<'a> = State<'a, TokenHydrators>;

#[tauri::command]
pub async fn session_submit(
    adapter: AdapterState<'_>,
    hydrators: HydratorsState<'_>,
    text: String,
    #[allow(non_snake_case)] attachments: Option<Vec<Attachment>>,
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
    let hydrated = hydrators.hydrate_all(&text);
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
/// before the post-restart actor spawns. Drives the K-266 cwd palette.
#[tauri::command]
pub async fn instance_restart(
    adapter: AdapterState<'_>,
    instance_id: String,
    cwd: Option<PathBuf>,
) -> Result<Value, String> {
    tracing::info!(instance_id = %instance_id, cwd = ?cwd, "cmd::instance_restart: entry");
    let key = InstanceKey::parse(&instance_id).map_err(|e| e.to_string())?;
    let out = adapter.restart_instance(key, cwd).await.map_err(|e| e.message);
    match &out {
        Ok(_) => tracing::info!("cmd::instance_restart: accepted"),
        Err(err) => tracing::warn!(%err, "cmd::instance_restart: failed"),
    }
    out.map(|key| serde_json::json!({ "id": key.as_string() }))
}

#[tauri::command]
pub async fn agents_list(adapter: AdapterState<'_>) -> Result<Value, String> {
    Ok(serde_json::json!({ "agents": adapter.list_agents() }))
}

#[tauri::command]
pub async fn profiles_list(adapter: AdapterState<'_>) -> Result<Value, String> {
    Ok(serde_json::json!({ "profiles": adapter.list_profiles() }))
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
        let agent_id = cfg
            .agents
            .agent
            .default
            .clone()
            .or_else(|| cfg.agents.agents.first().map(|a| a.id.clone()))
            .unwrap_or_default();
        (agent_id, cfg.profile.default.clone())
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
) -> Result<(), String> {
    tracing::info!(
        instance_id = ?instance_id,
        agent_id = ?agent_id,
        profile_id = ?profile_id,
        session_id = %session_id,
        "cmd::session_load: entry"
    );
    let out = adapter
        .load_session(
            instance_id.as_deref(),
            agent_id.as_deref(),
            profile_id.as_deref(),
            session_id,
        )
        .await
        .map_err(|e| e.message);
    match &out {
        Ok(_) => tracing::info!("cmd::session_load: accepted"),
        Err(err) => tracing::warn!(%err, "cmd::session_load: failed"),
    }
    out
}

/// List every live instance the adapter knows about. Mirrors the
/// `instances/list` JSON-RPC method; used by the K-274 instances
/// palette leaf to drive its row list. Returns the same shape the
/// JSON-RPC handler emits so UI code reading either surface treats
/// them uniformly.
#[tauri::command]
pub async fn instances_list(adapter: AdapterState<'_>) -> Result<Value, String> {
    let items = adapter.list().await;
    let wire: Vec<Value> = items
        .iter()
        .map(|i| {
            json!({
                "agentId": i.agent_id,
                "profileId": i.profile_id,
                "instanceId": i.id,
                "sessionId": i.session_id,
                "mode": i.mode,
            })
        })
        .collect();
    Ok(json!({ "instances": wire }))
}

#[tauri::command]
pub async fn instances_focus(adapter: AdapterState<'_>, id: String) -> Result<Value, String> {
    let key = InstanceKey::parse(&id).map_err(|e| e.to_string())?;
    let key = adapter.focus(key).await.map_err(|e| e.to_string())?;
    Ok(json!({ "focusedId": key.as_string() }))
}

#[tauri::command]
pub async fn instances_shutdown(adapter: AdapterState<'_>, id: String) -> Result<Value, String> {
    let key = InstanceKey::parse(&id).map_err(|e| e.to_string())?;
    let key = adapter.shutdown_one(key).await.map_err(|e| e.to_string())?;
    Ok(json!({ "id": key.as_string() }))
}

/// Rename a live instance. `id` accepts UUID or current name; `name`
/// is `None` (clear) or a slug-validated string. The actual slug
/// validation runs inside `Adapter::rename` so the wire shape stays
/// consistent with the RPC handler.
#[tauri::command]
pub async fn instances_rename(adapter: AdapterState<'_>, id: String, name: Option<String>) -> Result<Value, String> {
    let key = adapter
        .resolve_token(&id)
        .await
        .ok_or_else(|| format!("instance '{id}' not found"))?;
    adapter.rename(key, name.clone()).await.map_err(|e| e.to_string())?;
    Ok(json!({
        "instanceId": key.as_string(),
        "name": name,
    }))
}

/// Switch the active model for the addressed instance. Today
/// returns the same `-32603`-shaped error the `models/set` wire
/// handler does — `AcpAdapter::set_session_model` stubs past the
/// membership check until K-251 wires the runtime side. The UI
/// surfaces the message via toast.
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
/// Mirrors `models_set` — stubbed at the adapter until K-251.
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
pub async fn instance_meta(adapter: AdapterState<'_>, instance_id: String) -> Result<Value, String> {
    tracing::debug!(instance_id = %instance_id, "cmd::instance_meta: entry");
    adapter.instance_meta(&instance_id).await.map_err(|e| e.message)
}

/// Resolve a pending permission prompt with the captain's pick.
///
/// `option_id` MUST be one of the agent-offered option ids — the wire
/// no longer carries synthetic `'allow'` / `'deny'` shortcuts. The
/// captain's "remember this" intent rides on the option's typed
/// `kind` field (`allow_always` / `reject_always`) — the controller's
/// `respond` method reads that and writes the trust-store entry
/// atomically before signaling the waiter.
///
/// No-op when no waiter matches `request_id` (already resolved, timed
/// out, or never registered). The command never errors on that path —
/// the UI sees `Ok(())` regardless so a stale reply doesn't surface
/// as a user-visible failure.
#[tauri::command]
pub async fn permission_reply(
    controller: State<'_, Arc<dyn PermissionController>>,
    _session_id: String,
    request_id: String,
    option_id: String,
) -> Result<(), String> {
    tracing::info!(
        request_id = %request_id,
        option_id = %option_id,
        "cmd::permission_reply: entry"
    );
    match controller.respond(&request_id, &option_id).await {
        None => {
            tracing::debug!(request_id, "permission_reply: no waiter — no-op");
        }
        Some(false) => {
            tracing::warn!(
                request_id,
                option_id,
                "permission_reply: option_id not in offered set — no-op"
            );
        }
        Some(true) => {
            tracing::info!(request_id, option_id, "cmd::permission_reply: resolved");
        }
    }
    Ok(())
}

/// Snapshot of the runtime trust store filtered to the addressed
/// instance. Drives the permissions palette so the captain can review
/// the live `(tool, decision)` set + prune entries that no longer fit
/// (a tool flipped to "always allow" mid-session that should now ask
/// again, etc.). Empty list when no rules match. Decision is the
/// camelCase wire form (`allow` / `deny`).
#[tauri::command]
pub async fn permissions_trust_snapshot(
    controller: State<'_, Arc<dyn PermissionController>>,
    instance_id: String,
) -> Result<Value, String> {
    let snapshot = controller.snapshot_trust_store().await;
    let entries: Vec<Value> = snapshot
        .into_iter()
        .filter(|(iid, _, _)| iid == &instance_id)
        .map(|(_, tool, decision)| {
            serde_json::json!({
                "tool": tool,
                "decision": match decision {
                    TrustDecision::Allow => "allow",
                    TrustDecision::Deny => "deny",
                },
            })
        })
        .collect();
    Ok(serde_json::json!({ "entries": entries }))
}

/// Drop a single trust-store entry. Captain-driven — paired with the
/// permissions palette's multi-select toggle so unticking a row
/// removes the rule. No-op when the entry isn't present (idempotent
/// against double-clicks / palette reuse).
#[tauri::command]
pub async fn permissions_trust_forget(
    controller: State<'_, Arc<dyn PermissionController>>,
    instance_id: String,
    tool: String,
) -> Result<(), String> {
    controller.forget_trust(&instance_id, &tool).await;
    Ok(())
}

/// Read-only snapshot of the resolved MCP set. UI's palette `mcps`
/// leaf binds to this. With per-instance overrides gone (S5), every
/// server in the resolved file set is "active"; captains can't
/// toggle one off without editing the JSON files + `daemon/reload`.
/// The returned shape passes `raw` through verbatim so the UI's
/// preview pane can render the full opaque entry — env values are
/// NOT redacted here (UI does the redaction layer).
#[tauri::command]
pub async fn mcps_list(mcps: MCPsState<'_>) -> Result<Value, String> {
    let catalog = mcps.list();
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
                "source": m.source.display().to_string(),
            })
        })
        .collect();
    Ok(serde_json::json!({ "mcps": items }))
}
