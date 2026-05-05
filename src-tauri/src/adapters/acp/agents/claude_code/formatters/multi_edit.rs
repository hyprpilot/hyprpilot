//! claude-code's `MultiEdit` tool — N atomic edits to a single file
//! in one call. Title shares `edit · <path>`; stat surfaces edit count.

use serde_json::Value;

use crate::tools::formatter::registry::{FormatterContext, FormatterRegistry, ToolFormatter};
use crate::tools::formatter::shared::{format_diff_hunk, pick, short_path, text_blocks};
use crate::tools::formatter::types::FormattedToolCall;

pub struct MultiEditFormatter;

impl ToolFormatter for MultiEditFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let path = pick::<String>(ctx.raw_input, "file_path");
        let edits = pick::<Vec<Value>>(ctx.raw_input, "edits").unwrap_or_default();
        let count = edits.len();

        let title = match path.as_deref() {
            Some(p) => format!("edit · {}", short_path(p)),
            None => "edit".to_string(),
        };
        let stat = if count > 0 {
            Some(format!("{} {}", count, if count == 1 { "edit" } else { "edits" }))
        } else {
            None
        };

        // Stack one fence per edit. Each gets the file's language
        // (rich per-language highlight + transformerNotationDiff
        // markers) when the extension resolves; otherwise drops to
        // a `\`\`\`diff` fence with `+`/`-` prefixes.
        let mut hunks: Vec<String> = Vec::new();
        for edit in &edits {
            let old_text = edit.get("old_string").and_then(|v| v.as_str()).unwrap_or("");
            let new_text = edit.get("new_string").and_then(|v| v.as_str()).unwrap_or("");
            if let Some(hunk) = format_diff_hunk(path.as_deref(), old_text, new_text) {
                hunks.push(hunk);
            }
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
            stat,
            description,
            output,
            fields: Vec::new(),
        }
    }
}

pub fn register(reg: &mut FormatterRegistry, adapter: &str) {
    reg.register_adapter(adapter, "MultiEdit", Box::new(MultiEditFormatter));
}
