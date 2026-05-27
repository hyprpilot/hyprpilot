//! codex-acp's `Edit` patch-apply tool. RawInput carries
//! `PatchApplyBeginEvent` / `PatchApplyUpdatedEvent` /
//! `ApplyPatchApprovalRequestEvent` — `changes` is a map of
//! `path → FileChange { Add | Delete | Update { unified_diff,
//! move_path? } }`. Permission-time requests also carry ACP `diff`
//! content blocks. Render both markdown and plain `diff` surfaces so
//! every frontend can show the patch.

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::{format_diff_hunk, format_git_diff, pick};
use crate::tools::formatter::types::{FormattedToolCall, Stat, ToolField};

pub struct EditFormatter;

impl ToolFormatter for EditFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let title = if ctx.wire_name.trim().is_empty() {
            "edit".to_string()
        } else {
            ctx.wire_name.trim().to_string()
        };

        let mut fields: Vec<ToolField> = Vec::new();
        if let Some(map) = ctx.raw_input.and_then(|v| v.get("changes")).and_then(|v| v.as_object()) {
            for (path, change) in map {
                let action = file_change_label(change);
                fields.push(ToolField {
                    label: action.into(),
                    value: path.clone(),
                });
            }
        }
        if let Some(auto) = pick::<bool>(ctx.raw_input, "auto_approved") {
            fields.push(ToolField {
                label: "auto approved".into(),
                value: auto.to_string(),
            });
        }

        let rendered = render_content_diffs(ctx.content).or_else(|| render_raw_changes(ctx.raw_input));
        let (description, diff, stats) = rendered.unwrap_or((None, None, Vec::new()));

        FormattedToolCall {
            title,
            stats,
            description,
            diff,
            output: None,
            fields,
        }
    }
}

/// `FileChange` is `{ "Add": {...} }` / `{ "Delete": {...} }` /
/// `{ "Update": {..., move_path? } }`. We label by the first key.
fn file_change_label(change: &serde_json::Value) -> &'static str {
    match change.as_object().and_then(|m| m.keys().next()).map(String::as_str) {
        Some("Add") => "add",
        Some("Delete") => "delete",
        Some("Update") => "update",
        _ => "change",
    }
}

type RenderedDiff = (Option<String>, Option<String>, Vec<Stat>);

fn render_content_diffs(content: &[serde_json::Value]) -> Option<RenderedDiff> {
    let mut descriptions = Vec::new();
    let mut diffs = Vec::new();
    let mut added = 0;
    let mut removed = 0;

    for block in content {
        if block.get("type").and_then(|v| v.as_str()) != Some("diff") {
            continue;
        }
        let path = block.get("path").and_then(|v| v.as_str());
        let old_text = block.get("oldText").and_then(|v| v.as_str()).unwrap_or("");
        let new_text = block.get("newText").and_then(|v| v.as_str()).unwrap_or("");
        added += new_text.lines().count() as u32;
        removed += old_text.lines().count() as u32;

        if let Some(markdown) = format_diff_hunk(path, old_text, new_text) {
            descriptions.push(markdown);
        }
        if let Some(patch) = format_git_diff(path, old_text, new_text) {
            diffs.push(patch);
        }
    }

    rendered(descriptions, diffs, added, removed)
}

fn render_raw_changes(raw: Option<&serde_json::Value>) -> Option<RenderedDiff> {
    let map = raw?.get("changes")?.as_object()?;
    let mut descriptions = Vec::new();
    let mut diffs = Vec::new();
    let mut added = 0;
    let mut removed = 0;

    for (path, change) in map {
        if let Some(diff) = file_change_unified_diff(change) {
            let (diff_added, diff_removed) = count_unified_diff(diff);
            added += diff_added;
            removed += diff_removed;
            descriptions.push(format!("`{path}`\n\n```diff\n{diff}\n```"));
            diffs.push(diff.to_string());

            continue;
        }

        let Some((old_text, new_text)) = file_change_text(change) else {
            continue;
        };
        added += new_text.lines().count() as u32;
        removed += old_text.lines().count() as u32;
        if let Some(markdown) = format_diff_hunk(Some(path), old_text, new_text) {
            descriptions.push(markdown);
        }
        if let Some(patch) = format_git_diff(Some(path), old_text, new_text) {
            diffs.push(patch);
        }
    }

    rendered(descriptions, diffs, added, removed)
}

fn file_change_text(change: &serde_json::Value) -> Option<(&str, &str)> {
    let obj = change.as_object()?;
    if let Some(add) = obj.get("Add") {
        return Some(("", add.get("content").and_then(|v| v.as_str()).unwrap_or("")));
    }
    if let Some(delete) = obj.get("Delete") {
        return Some((delete.get("content").and_then(|v| v.as_str()).unwrap_or(""), ""));
    }
    if let Some(update) = obj.get("Update") {
        let diff = update.get("unified_diff").and_then(|v| v.as_str()).unwrap_or("");

        return Some((diff, diff));
    }

    None
}

fn file_change_unified_diff(change: &serde_json::Value) -> Option<&str> {
    let diff = change.as_object()?.get("Update")?.get("unified_diff")?.as_str()?.trim();

    (!diff.is_empty()).then_some(diff)
}

fn count_unified_diff(diff: &str) -> (u32, u32) {
    let mut added = 0;
    let mut removed = 0;

    for line in diff.lines() {
        if line.starts_with("+++") || line.starts_with("---") {
            continue;
        }
        if line.starts_with('+') {
            added += 1;
        } else if line.starts_with('-') {
            removed += 1;
        }
    }

    (added, removed)
}

fn rendered(descriptions: Vec<String>, diffs: Vec<String>, added: u32, removed: u32) -> Option<RenderedDiff> {
    if descriptions.is_empty() && diffs.is_empty() {
        return None;
    }
    let stats = if added == 0 && removed == 0 {
        Vec::new()
    } else {
        vec![Stat::Diff { added, removed }]
    };
    let description = (!descriptions.is_empty()).then(|| descriptions.join("\n\n"));
    let diff = (!diffs.is_empty()).then(|| diffs.join("\n"));

    Some((description, diff, stats))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::EditFormatter;
    use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};

    #[test]
    fn content_diff_blocks_populate_diff_surfaces() {
        let content = vec![json!({
            "type": "diff",
            "path": "src/main.rs",
            "oldText": "fn main() {}\n",
            "newText": "fn main() {\n    println!(\"hi\");\n}\n"
        })];
        let ctx = FormatterContext {
            wire_name: "Edit src/main.rs",
            tool_kind: &crate::tools::ToolKind::Edit,
            raw_input: None,
            adapter: "acp-codex",
            content: &content,
            started_at: 0,
            completed_at: None,
        };
        let formatted = EditFormatter.format(&ctx);

        assert!(formatted.description.as_deref().unwrap_or_default().contains("```rust"));
        assert!(formatted.diff.as_deref().unwrap_or_default().contains("diff --git"));
        assert_eq!(
            formatted.stats,
            vec![crate::tools::formatter::types::Stat::Diff { added: 3, removed: 1 }]
        );
    }

    #[test]
    fn raw_update_unified_diff_is_not_diffed_against_itself() {
        let raw = json!({
            "changes": {
                "src/main.rs": {
                    "Update": {
                        "unified_diff": "@@ -1 +1 @@\n-old\n+new\n"
                    }
                }
            }
        });
        let ctx = FormatterContext {
            wire_name: "Edit src/main.rs",
            tool_kind: &crate::tools::ToolKind::Edit,
            raw_input: Some(&raw),
            adapter: "acp-codex",
            content: &[],
            started_at: 0,
            completed_at: None,
        };
        let formatted = EditFormatter.format(&ctx);

        assert_eq!(formatted.diff.as_deref(), Some("@@ -1 +1 @@\n-old\n+new"));
        assert_eq!(
            formatted.stats,
            vec![crate::tools::formatter::types::Stat::Diff { added: 1, removed: 1 }]
        );
    }
}
