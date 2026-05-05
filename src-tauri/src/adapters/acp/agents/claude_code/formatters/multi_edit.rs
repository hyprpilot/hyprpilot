//! claude-code's `MultiEdit` tool — N atomic edits to a single file
//! in one call. Title shares `edit · <path>`; stat surfaces edit count.

use serde_json::Value;

use crate::tools::formatter::registry::{FormatterContext, FormatterRegistry, ToolFormatter};
use crate::tools::formatter::shared::{format_diff_hunk, line_magnitudes, pick, short_path, text_blocks};
use crate::tools::formatter::types::{FormattedToolCall, Stat};

pub struct MultiEditFormatter;

impl ToolFormatter for MultiEditFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let path = pick::<String>(ctx.raw_input, "file_path");
        let edits = pick::<Vec<Value>>(ctx.raw_input, "edits").unwrap_or_default();

        let title = match path.as_deref() {
            Some(p) => format!("edit · {}", short_path(p)),
            None => "edit".to_string(),
        };

        // Sum diff stats across every edit. Each edit's
        // `(old_string, new_string)` contributes to the running
        // (added, removed) total — captain reads the magnitude of
        // the whole multi-edit at a glance.
        let mut total_added: u32 = 0;
        let mut total_removed: u32 = 0;
        let mut hunks: Vec<String> = Vec::new();
        for edit in &edits {
            let old_text = edit.get("old_string").and_then(|v| v.as_str()).unwrap_or("");
            let new_text = edit.get("new_string").and_then(|v| v.as_str()).unwrap_or("");
            let (a, r) = line_magnitudes(old_text, new_text);
            total_added = total_added.saturating_add(a);
            total_removed = total_removed.saturating_add(r);
            if let Some(hunk) = format_diff_hunk(path.as_deref(), old_text, new_text) {
                hunks.push(hunk);
            }
        }

        let mut stats: Vec<Stat> = Vec::new();
        if total_added > 0 || total_removed > 0 {
            stats.push(Stat::Diff {
                added: total_added,
                removed: total_removed,
            });
        }

        let description = if hunks.is_empty() {
            None
        } else {
            Some(hunks.join("\n\n"))
        };

        let output_text = text_blocks(ctx.content);
        let output = if output_text.is_empty() {
            None
        } else {
            Some(output_text)
        };

        FormattedToolCall {
            title,
            stats,
            description,
            output,
            fields: Vec::new(),
        }
    }
}

pub fn register(reg: &mut FormatterRegistry, adapter: &str) {
    reg.register_adapter(adapter, "MultiEdit", Box::new(MultiEditFormatter));
}
