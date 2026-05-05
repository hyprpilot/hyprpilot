//! claude-code's `Edit` tool. Title surfaces the path + replace-all
//! flag. Diff content blocks (`{type:"diff", oldText, newText}`)
//! render as labeled before/after fences in the description so the
//! captain reads the change inline.

use crate::tools::formatter::registry::{FormatterContext, FormatterRegistry, ToolFormatter};
use crate::tools::formatter::shared::{format_diff_hunk, pick, short_path, text_blocks, wire_title_or_fallback};
use crate::tools::formatter::types::FormattedToolCall;

pub struct EditFormatter;

impl ToolFormatter for EditFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let path = pick::<String>(ctx.raw_input, "file_path");
        let replace_all = pick::<bool>(ctx.raw_input, "replace_all").unwrap_or(false);

        let prefix = wire_title_or_fallback(ctx.wire_name, "Edit");
        let title = match path.as_deref() {
            Some(p) => {
                let trimmed = short_path(p);
                if replace_all {
                    format!("{} · {} (replace all)", prefix, trimmed)
                } else {
                    format!("{} · {}", prefix, trimmed)
                }
            }
            None => prefix,
        };

        // Prefer agent-supplied diff content blocks (populated as the
        // tool runs); fall back to the rawInput's `old_string` /
        // `new_string` so the captain sees the impending change at
        // permission time, before content blocks are streamed.
        let description = render_diff_blocks(ctx.content, path.as_deref()).or_else(|| {
            let old_text = pick::<String>(ctx.raw_input, "old_string").unwrap_or_default();
            let new_text = pick::<String>(ctx.raw_input, "new_string").unwrap_or_default();
            format_diff_hunk(path.as_deref(), &old_text, &new_text)
        });

        let output_text = text_blocks(ctx.content);
        let output = if output_text.is_empty() {
            None
        } else {
            Some(output_text)
        };

        FormattedToolCall {
            title,
            stat: None,
            description,
            output,
            fields: Vec::new(),
        }
    }
}

/// Project diff content blocks into Shiki-friendly markdown fences.
/// Per-block path lets each block infer its own language; falls back
/// to the tool-level `path` arg when the block omits one.
fn render_diff_blocks(content: &[serde_json::Value], fallback_path: Option<&str>) -> Option<String> {
    let mut hunks: Vec<String> = Vec::new();
    for block in content {
        if block.get("type").and_then(|v| v.as_str()) != Some("diff") {
            continue;
        }
        let new_text = block.get("newText").and_then(|v| v.as_str()).unwrap_or("");
        let old_text = block.get("oldText").and_then(|v| v.as_str()).unwrap_or("");
        let block_path = block.get("path").and_then(|v| v.as_str()).or(fallback_path);
        if let Some(hunk) = format_diff_hunk(block_path, old_text, new_text) {
            hunks.push(hunk);
        }
    }
    if hunks.is_empty() {
        None
    } else {
        Some(hunks.join("\n\n"))
    }
}

pub fn register(reg: &mut FormatterRegistry, adapter: &str) {
    reg.register_adapter(adapter, "Edit", Box::new(EditFormatter));
}
