//! Tauri `#[command]`s the webview invokes: `acp_submit`, `acp_cancel`,
//! `permission_reply`, `agents_list`, `profiles_list`, `session_list`,
//! `session_load`. Each delegates into the shared `AcpInstances`
//! registry; the RPC surface uses the same entry points.

use std::path::PathBuf;
use std::sync::Arc;

use agent_client_protocol::schema::ListSessionsResponse;
use serde_json::Value;
use tauri::State;

use super::AcpInstances;

#[tauri::command]
pub async fn acp_submit(
    instances: State<'_, Arc<AcpInstances>>,
    text: String,
    agent_id: Option<String>,
    profile_id: Option<String>,
) -> Result<Value, String> {
    instances
        .submit(&text, agent_id.as_deref(), profile_id.as_deref())
        .await
        .map_err(|e| e.message)
}

#[tauri::command]
pub async fn acp_cancel(instances: State<'_, Arc<AcpInstances>>, agent_id: Option<String>) -> Result<Value, String> {
    instances.cancel(agent_id.as_deref()).await.map_err(|e| e.message)
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
    instances
        .list(agent_id.as_deref(), profile_id.as_deref(), cwd)
        .await
        .map_err(|e| e.message)
}

#[tauri::command]
pub async fn session_load(
    instances: State<'_, Arc<AcpInstances>>,
    agent_id: Option<String>,
    profile_id: Option<String>,
    session_id: String,
) -> Result<(), String> {
    instances
        .load(agent_id.as_deref(), profile_id.as_deref(), session_id)
        .await
        .map_err(|e| e.message)
}

/// Stub — permission replies route through a future
/// `PermissionController` issue (K-6 per the CLAUDE.md split).
/// Server auto-`Cancelled` is the current policy, so the webview
/// should not call this today; panic if it does.
#[tauri::command]
pub fn permission_reply(_session_id: String, _request_id: String, _option_id: String) -> Result<(), String> {
    unimplemented!("permission_reply: PermissionController not yet implemented (K-6 follow-up)");
}
