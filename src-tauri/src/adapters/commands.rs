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
use serde_json::Value;
use tauri::State;

use super::acp::AcpAdapter;
use super::permission::{pick_allow_option_id, pick_reject_option_id, PermissionController, PermissionOutcome};
use super::transcript::Attachment;
use super::instance::InstanceKey;
use crate::mcp::MCPsRegistry;

type AdapterState<'a> = State<'a, Arc<AcpAdapter>>;
type MCPsState<'a> = State<'a, Arc<MCPsRegistry>>;

#[tauri::command]
pub async fn session_submit(
    adapter: AdapterState<'_>,
    text: String,
    #[allow(non_snake_case)] attachments: Option<Vec<Attachment>>,
    instance_id: Option<String>,
    agent_id: Option<String>,
    profile_id: Option<String>,
) -> Result<Value, String> {
    let attachments = attachments.unwrap_or_default();
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

#[tauri::command]
pub async fn agents_list(adapter: AdapterState<'_>) -> Result<Value, String> {
    Ok(serde_json::json!({ "agents": adapter.list_agents() }))
}

#[tauri::command]
pub async fn commands_list(adapter: AdapterState<'_>, instance_id: String) -> Result<Value, String> {
    let commands = adapter.list_commands(&instance_id).await.map_err(|e| e.message)?;
    Ok(serde_json::json!({ "commands": commands }))
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

/// Switch the active model for the addressed instance. Today
/// returns the same `-32603`-shaped error the `models/set` wire
/// handler does — `AcpAdapter::set_session_model` stubs past the
/// membership check until K-251 wires the runtime side. The UI
/// surfaces the message via toast.
#[tauri::command]
pub async fn models_set(
    adapter: AdapterState<'_>,
    instance_id: String,
    model_id: String,
) -> Result<Value, String> {
    tracing::info!(instance_id = %instance_id, model_id = %model_id, "cmd::models_set: entry");
    let out = adapter.set_session_model(&instance_id, &model_id).await.map_err(|e| e.message);
    if let Err(err) = &out {
        tracing::warn!(%err, "cmd::models_set: failed");
    }
    out
}

/// Switch the active operational mode for the addressed instance.
/// Mirrors `models_set` — stubbed at the adapter until K-251.
#[tauri::command]
pub async fn modes_set(
    adapter: AdapterState<'_>,
    instance_id: String,
    mode_id: String,
) -> Result<Value, String> {
    tracing::info!(instance_id = %instance_id, mode_id = %mode_id, "cmd::modes_set: entry");
    let out = adapter.set_session_mode(&instance_id, &mode_id).await.map_err(|e| e.message);
    if let Err(err) = &out {
        tracing::warn!(%err, "cmd::modes_set: failed");
    }
    out
}

/// Resolve a pending permission prompt. The UI sends one of:
///
/// - `"allow"` — the controller picks the best allow-kind option id
///   from the original options[] stashed at register_pending time.
/// - `"deny"` — mapped to Cancelled (falls through to a vendor
///   reject option when one is present; otherwise the ACP wire sees
///   `Cancelled` directly — see pick_reject_option_id).
/// - any other string — treated as a raw ACP option id. The
///   PermissionController routes it verbatim; the ACP client wraps
///   it into Selected(option_id).
///
/// No-op when no waiter matches `request_id` (already resolved, timed
/// out, or never registered). The command never errors on that path —
/// the UI sees `Ok(())` regardless so a stale reply doesn't surface
/// as a user-visible failure.
// TODO: the bare `allow` / `deny` tokens shadow any vendor option_id
// that happens to use those literals. Namespace as `hyp:allow` /
// `hyp:deny` or promote to an explicit enum on the Tauri command — a
// real fix cross-cuts !36 (UI-side senders) so this lands in a
// follow-up.
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
    let controller = controller.inner().clone();
    let outcome = match option_id.as_str() {
        "allow" => {
            let Some(options) = controller.options_for(&request_id).await else {
                tracing::debug!(request_id, "permission_reply: no waiter (allow) — no-op");
                return Ok(());
            };
            match pick_allow_option_id(&options) {
                Some(id) => PermissionOutcome::Selected(id),
                None => PermissionOutcome::Cancelled,
            }
        }
        "deny" => {
            let Some(options) = controller.options_for(&request_id).await else {
                tracing::debug!(request_id, "permission_reply: no waiter (deny) — no-op");
                return Ok(());
            };
            match pick_reject_option_id(&options) {
                Some(id) => PermissionOutcome::Selected(id),
                None => PermissionOutcome::Cancelled,
            }
        }
        raw => PermissionOutcome::Selected(raw.to_string()),
    };
    tracing::info!(
        request_id = %request_id,
        outcome = ?outcome,
        "cmd::permission_reply: resolved"
    );
    controller.resolve(&request_id, outcome).await;
    Ok(())
}

/// `mcps_list` — return the global catalog with a per-instance
/// `enabled` bit. When `instance_id` resolves to a live actor the bit
/// reflects the override-or-profile-default; otherwise everything is
/// enabled (the daemon-wide "no filter" semantics).
#[tauri::command]
pub async fn mcps_list(
    adapter: AdapterState<'_>,
    mcps: MCPsState<'_>,
    instance_id: Option<String>,
) -> Result<Value, String> {
    let catalog = mcps.list();
    let enabled_filter = match instance_id.as_deref() {
        Some(id) => {
            let key = InstanceKey::parse(id).map_err(|err| format!("mcps_list instance_id '{id}': {err}"))?;
            Some(adapter.effective_mcps_for(key).await)
        }
        None => None,
    };
    let items: Vec<Value> = catalog
        .iter()
        .map(|m| {
            let enabled = match &enabled_filter {
                None => true,
                Some(None) => true,
                Some(Some(list)) => list.iter().any(|n| n == &m.name),
            };
            serde_json::json!({
                "name": m.name,
                "command": m.command,
                "enabled": enabled,
            })
        })
        .collect();
    Ok(serde_json::json!({ "mcps": items }))
}

/// `mcps_set` — install a per-instance enabled-list override and
/// trigger an instance restart. Mirrors the `mcps/set` socket RPC.
#[tauri::command]
pub async fn mcps_set(
    adapter: AdapterState<'_>,
    mcps: MCPsState<'_>,
    instance_id: String,
    enabled: Vec<String>,
) -> Result<Value, String> {
    let key = InstanceKey::parse(&instance_id).map_err(|err| format!("mcps_set instance_id '{instance_id}': {err}"))?;
    let _ = adapter.contains_instance(&instance_id).await.map_err(|e| e.message)?;
    let catalog = mcps.list();
    for name in &enabled {
        if !catalog.iter().any(|m| &m.name == name) {
            return Err(format!("mcps_set: '{name}' not in [[mcps]] catalog"));
        }
    }
    adapter.set_mcps_override(key, enabled);
    adapter.restart_instance(key).await.map_err(|e| e.message)?;
    Ok(serde_json::json!({ "restarted": true }))
}
