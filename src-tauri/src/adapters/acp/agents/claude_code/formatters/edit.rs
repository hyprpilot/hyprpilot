//! claude-code's `Edit` tool. Title surfaces the path + replace-all
//! flag. Diff content blocks (`{type:"diff", oldText, newText}`)
//! render as labeled before/after fences in the description so the
//! captain reads the change inline.

use crate::tools::formatter::registry::{FormatterContext, FormatterRegistry, ToolFormatter};
use crate::tools::formatter::shared::{format_diff_hunk, format_git_diff, pick, text_blocks, wire_title_or_fallback};
use crate::tools::formatter::types::FormattedToolCall;

pub struct EditFormatter;

impl ToolFormatter for EditFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let path = pick::<String>(ctx.raw_input, "file_path");
        let replace_all = pick::<bool>(ctx.raw_input, "replace_all").unwrap_or(false);

        let prefix = wire_title_or_fallback(ctx.wire_name, "Edit");
        let title = match path.as_deref() {
            Some(p) => {
                if replace_all {
                    format!("{} · {} (replace all)", prefix, p)
                } else {
                    format!("{} · {}", prefix, p)
                }
            }
            None => prefix,
        };

        // Prefer agent-supplied diff content blocks (populated as the
        // tool runs); fall back to the rawInput's `old_string` /
        // `new_string` so the captain sees the impending change at
        // permission time, before content blocks are streamed. Both
        // surfaces (Shiki-marker markdown + plain git-diff) are
        // produced in parallel from the same source pair, so they
        // can't go out of sync.
        let (description, diff) = render_diff_blocks(ctx.content, path.as_deref()).unwrap_or_else(|| {
            let old_text = pick::<String>(ctx.raw_input, "old_string").unwrap_or_default();
            let new_text = pick::<String>(ctx.raw_input, "new_string").unwrap_or_default();
            (
                format_diff_hunk(path.as_deref(), &old_text, &new_text),
                format_git_diff(path.as_deref(), &old_text, &new_text),
            )
        });

        let output_text = text_blocks(ctx.content);
        let output = if output_text.is_empty() {
            None
        } else {
            Some(output_text)
        };

        FormattedToolCall {
            title,
            stats: Vec::new(),
            description,
            diff,
            output,
            fields: Vec::new(),
        }
    }
}

/// Project diff content blocks into two parallel surfaces:
/// Shiki-marker markdown (`description`) AND unified git-diff
/// (`diff`). Per-block path lets each block infer its own
/// language / file header; falls back to the tool-level `path`
/// arg when the block omits one. Returns
/// `None` when no `diff`-typed content blocks are present so the
/// caller falls through to the rawInput-based path.
fn render_diff_blocks(
    content: &[serde_json::Value],
    fallback_path: Option<&str>,
) -> Option<(Option<String>, Option<String>)> {
    let mut hunks: Vec<String> = Vec::new();
    let mut git_hunks: Vec<String> = Vec::new();
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
        if let Some(g) = format_git_diff(block_path, old_text, new_text) {
            git_hunks.push(g);
        }
    }
    if hunks.is_empty() && git_hunks.is_empty() {
        return None;
    }
    let description = if hunks.is_empty() {
        None
    } else {
        Some(hunks.join("\n\n"))
    };
    let diff = if git_hunks.is_empty() {
        None
    } else {
        Some(git_hunks.join("\n"))
    };
    Some((description, diff))
}

pub fn register(reg: &mut FormatterRegistry, adapter: &str) {
    reg.register_adapter(adapter, "Edit", Box::new(EditFormatter));
}
