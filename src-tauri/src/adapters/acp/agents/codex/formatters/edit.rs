//! codex-acp's `Edit` patch-apply tool. RawInput carries
//! `PatchApplyBeginEvent` / `PatchApplyUpdatedEvent` /
//! `ApplyPatchApprovalRequestEvent` — `changes` is a map of
//! `path → FileChange { Add | Delete | Update { unified_diff,
//! move_path? } }`. Title is already a comma-joined path list; we
//! pass it through and surface the change set as fields.

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::pick;
use crate::tools::formatter::types::{FormattedToolCall, ToolField};

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

        FormattedToolCall {
            title,
            stats: Vec::new(),
            description: None,
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
