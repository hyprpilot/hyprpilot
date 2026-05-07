//! Tauri `#[command]`s for the per-instance skills catalogue.
//!
//! Skills are owned by the `AcpInstance`, not the daemon. Each
//! command takes an optional `instance_id`; when provided, the
//! addressed instance's registry serves the call. When omitted,
//! the focused instance is used. When no instance is focused
//! (boot pre-spawn / all-shutdown), the listing is empty — the
//! palette opens silently with zero entries until the captain
//! spawns.

use std::sync::Arc;

use serde_json::{json, Value};
use tauri::State;

use super::{SkillSlug, SkillSummary, SkillsRegistry};
use crate::adapters::{AcpAdapter, InstanceKey};

type AdapterState<'a> = State<'a, Arc<AcpAdapter>>;

#[tauri::command]
pub async fn skills_list(adapter: AdapterState<'_>, instance_id: Option<String>) -> Result<Value, String> {
    let registry = resolve_registry(&adapter, instance_id.as_deref()).await;
    let list: Vec<SkillSummary> = match registry {
        Some(reg) => reg.list().iter().map(SkillSummary::from).collect(),
        None => Vec::new(),
    };
    Ok(json!({ "skills": list }))
}

#[tauri::command]
pub async fn skills_reload(adapter: AdapterState<'_>, instance_id: Option<String>) -> Result<Value, String> {
    let Some(reg) = resolve_registry(&adapter, instance_id.as_deref()).await else {
        return Ok(json!({ "count": 0, "skills": [] }));
    };
    reg.reload().map_err(|e| format!("skills reload failed: {e:#}"))?;
    let list: Vec<SkillSummary> = reg.list().iter().map(SkillSummary::from).collect();
    Ok(json!({ "count": list.len(), "skills": list }))
}

#[tauri::command]
pub async fn skills_get(adapter: AdapterState<'_>, instance_id: Option<String>, slug: String) -> Result<Value, String> {
    let parsed = SkillSlug::parse(&slug).map_err(|e| format!("invalid slug '{slug}': {e}"))?;
    let Some(reg) = resolve_registry(&adapter, instance_id.as_deref()).await else {
        return Err(format!("no live skills registry for slug '{slug}'"));
    };
    let Some(skill) = reg.get(&parsed) else {
        return Err(format!("unknown skill '{slug}'"));
    };
    Ok(json!({
        "slug": skill.slug,
        "title": skill.title,
        "description": skill.description,
        "body": skill.body,
        "path": skill.path.display().to_string(),
        "references": skill.references,
    }))
}

/// `instance_id` (when provided) addresses a specific instance; an
/// invalid or shut-down id collapses to `None` so the palette stays
/// silent rather than erroring at the captain mid-typing. With no id
/// the focused instance serves the call.
async fn resolve_registry(adapter: &Arc<AcpAdapter>, instance_id: Option<&str>) -> Option<Arc<SkillsRegistry>> {
    if let Some(raw) = instance_id {
        let key = InstanceKey::parse(raw).ok()?;
        return adapter.instance_skills(key).await;
    }
    adapter.focused_skills().await
}
