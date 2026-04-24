//! Tauri `#[command]`s live at the generic adapter layer (not under
//! `acp/`) because they dispatch through the single `Adapter` impl
//! today and will route via `dyn Adapter` once a second transport lands.

use std::path::PathBuf;
use std::sync::Arc;

use agent_client_protocol::schema::ListSessionsResponse;
use serde_json::Value;
use tauri::State;

use super::acp::AcpInstances;
use super::permission::{pick_allow_option_id, pick_reject_option_id, PermissionController, PermissionOutcome};

#[tauri::command]
pub async fn session_submit(
    instances: State<'_, Arc<AcpInstances>>,
    text: String,
    agent_id: Option<String>,
    profile_id: Option<String>,
) -> Result<Value, String> {
    tracing::info!(
        text_len = text.len(),
        agent_id = ?agent_id,
        profile_id = ?profile_id,
        "cmd::session_submit: entry"
    );
    let out = instances
        .submit(&text, agent_id.as_deref(), profile_id.as_deref())
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
    instances: State<'_, Arc<AcpInstances>>,
    agent_id: Option<String>,
) -> Result<Value, String> {
    tracing::info!(agent_id = ?agent_id, "cmd::session_cancel: entry");
    let out = instances.cancel(agent_id.as_deref()).await.map_err(|e| e.message);
    if let Err(err) = &out {
        tracing::warn!(%err, "cmd::session_cancel: failed");
    }
    out
}

#[tauri::command]
pub async fn agents_list(instances: State<'_, Arc<AcpInstances>>) -> Result<Value, String> {
    Ok(serde_json::json!({ "agents": instances.list_agents() }))
}

#[tauri::command]
pub async fn profiles_list(instances: State<'_, Arc<AcpInstances>>) -> Result<Value, String> {
    Ok(serde_json::json!({ "profiles": instances.list_profiles() }))
}

#[tauri::command]
pub async fn session_list(
    instances: State<'_, Arc<AcpInstances>>,
    agent_id: Option<String>,
    profile_id: Option<String>,
    cwd: Option<PathBuf>,
) -> Result<ListSessionsResponse, String> {
    tracing::info!(
        agent_id = ?agent_id,
        profile_id = ?profile_id,
        cwd = ?cwd,
        "cmd::session_list: entry"
    );
    let out = instances
        .list(agent_id.as_deref(), profile_id.as_deref(), cwd)
        .await
        .map_err(|e| e.message);
    if let Err(err) = &out {
        tracing::warn!(%err, "cmd::session_list: failed");
    }
    out
}

#[tauri::command]
pub async fn session_load(
    instances: State<'_, Arc<AcpInstances>>,
    agent_id: Option<String>,
    profile_id: Option<String>,
    session_id: String,
) -> Result<(), String> {
    tracing::info!(
        agent_id = ?agent_id,
        profile_id = ?profile_id,
        session_id = %session_id,
        "cmd::session_load: entry"
    );
    let out = instances
        .load(agent_id.as_deref(), profile_id.as_deref(), session_id)
        .await
        .map_err(|e| e.message);
    match &out {
        Ok(_) => tracing::info!("cmd::session_load: accepted"),
        Err(err) => tracing::warn!(%err, "cmd::session_load: failed"),
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
