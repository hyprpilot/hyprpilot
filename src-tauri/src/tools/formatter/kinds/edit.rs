//! `edit` — file modifications. RawInput convention is `file_path` /
//! `path`. Diff content (`{type:"diff", path, oldText, newText}`)
//! drops into the `description` via the shared `format_diff_hunk` —
//! per-language Shiki highlighting + `transformerNotationDiff`
//! markers when the path resolves to a known language; falls back to
//! a `\`\`\`diff` fence with `+`/`-` prefixes otherwise.

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::{args_to_fields, dedupe_output, format_diff_hunk, pick, wire_title_or_fallback};
use crate::tools::formatter::types::{FormattedToolCall, ToolField};

pub struct EditFormatter;

impl ToolFormatter for EditFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let path = pick::<String>(ctx.raw_input, "file_path")
            .or_else(|| pick(ctx.raw_input, "path"))
            .filter(|s| !s.is_empty());

        let title = wire_title_or_fallback(ctx.wire_name, "edit");

        let mut parts: Vec<String> = Vec::new();
        if let Some(d) = pick::<String>(ctx.raw_input, "description").filter(|s| !s.is_empty()) {
            parts.push(d);
        }
        parts.extend(diff_blocks_markdown(ctx.content, path.as_deref()));
        // Synthesise a diff from rawInput when no `{type:"diff"}` block
        // arrived yet (permission-time, pre-execute). Common shapes
        // across vendors: claude-code ships `old_string`/`new_string`,
        // opencode ships `oldString`/`newString`, write tools ship a
        // bare `content` (treat as "new file" — empty old).
        if parts.is_empty() {
            let old_text = pick::<String>(ctx.raw_input, "old_string")
                .or_else(|| pick(ctx.raw_input, "oldString"))
                .unwrap_or_default();
            let new_text = pick::<String>(ctx.raw_input, "new_string")
                .or_else(|| pick(ctx.raw_input, "newString"))
                .or_else(|| pick(ctx.raw_input, "content"))
                .unwrap_or_default();
            if let Some(hunk) = format_diff_hunk(path.as_deref(), &old_text, &new_text) {
                parts.push(hunk);
            }
        }
        let description = if parts.is_empty() {
            None
        } else {
            Some(parts.join("\n\n"))
        };

        // Edit-shape arg keys are consumed by the diff above — leaving
        // them in the fields grid would render them as redundant noise
        // (`OLD STRING` / `NEW STRING` / `CONTENT` rows beside an
        // already-rendered red/green diff).
        let mut fields = args_to_fields(
            ctx.raw_input,
            &[
                "file_path",
                "path",
                "description",
                "old_string",
                "new_string",
                "oldString",
                "newString",
                "content",
            ],
        );
        if let Some(p) = path {
            fields.insert(
                0,
                ToolField {
                    label: "path".into(),
                    value: p,
                },
            );
        }

        let output = dedupe_output(ctx.content, description.as_deref());

        FormattedToolCall {
            title,
            stat: None,
            description,
            output,
            fields,
        }
    }
}

/// Project diff content blocks into Shiki-friendly fences.
/// Per-block `path` field wins (vendors like to be explicit per
/// hunk); falls back to the tool-level path when omitted.
fn diff_blocks_markdown(content: &[serde_json::Value], fallback_path: Option<&str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for block in content {
        if block.get("type").and_then(|v| v.as_str()) != Some("diff") {
            continue;
        }
        let new_text = block.get("newText").and_then(|v| v.as_str()).unwrap_or("");
        let old_text = block.get("oldText").and_then(|v| v.as_str()).unwrap_or("");
        let block_path = block.get("path").and_then(|v| v.as_str()).or(fallback_path);
        if let Some(hunk) = format_diff_hunk(block_path, old_text, new_text) {
            out.push(hunk);
        }
    }
    out
}
