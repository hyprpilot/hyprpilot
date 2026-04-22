//! Tauri `#[command]`s the webview invokes: `acp_submit`, `acp_cancel`,
//! `permission_reply`, `agents_list`. Each delegates into the shared
//! `AcpSessions` registry; the RPC surface uses the same entry points.

use std::sync::Arc;

use serde_json::Value;
use tauri::State;

use super::AcpSessions;

#[tauri::command]
pub async fn acp_submit(
    sessions: State<'_, Arc<AcpSessions>>,
    text: String,
    agent_id: Option<String>,
) -> Result<Value, String> {
    sessions.submit(&text, agent_id.as_deref()).await.map_err(|e| e.message)
}

#[tauri::command]
pub async fn acp_cancel(sessions: State<'_, Arc<AcpSessions>>, agent_id: Option<String>) -> Result<Value, String> {
    sessions.cancel(agent_id.as_deref()).await.map_err(|e| e.message)
}

#[tauri::command]
pub async fn agents_list(sessions: State<'_, Arc<AcpSessions>>) -> Result<Value, String> {
    Ok(serde_json::json!({ "agents": sessions.list_agents() }))
}

/// Stub — permission replies route through a future
/// `PermissionController` issue (K-6 per the CLAUDE.md split).
/// Server auto-`Cancelled` is the current policy, so the webview
/// should not call this today; panic if it does.
#[tauri::command]
pub fn permission_reply(_session_id: String, _request_id: String, _option_id: String) -> Result<(), String> {
    unimplemented!("permission_reply: PermissionController not yet implemented (K-6 follow-up)");
}
