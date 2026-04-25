//! Tauri `#[command]`s mirroring the `skills/list` + `skills/get`
//! socket RPC. The webview palette (K-269) calls these to populate
//! the multi-select skills leaf and snapshot bodies into pending
//! attachments. Wire shape matches the socket handler in
//! `crate::rpc::handlers::skills`.

use std::sync::Arc;

use serde_json::{json, Value};
use tauri::State;

use super::{SkillSlug, SkillSummary, SkillsRegistry};

type SkillsState<'a> = State<'a, Arc<SkillsRegistry>>;

#[tauri::command]
pub async fn skills_list(skills: SkillsState<'_>) -> Result<Value, String> {
    let list: Vec<SkillSummary> = skills.list().iter().map(SkillSummary::from).collect();
    Ok(json!({ "skills": list }))
}

#[tauri::command]
pub async fn skills_get(skills: SkillsState<'_>, slug: String) -> Result<Value, String> {
    let parsed = SkillSlug::parse(&slug).map_err(|e| format!("invalid slug '{slug}': {e}"))?;
    let Some(skill) = skills.get(&parsed) else {
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
