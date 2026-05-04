//! opencode's `edit` tool. RawInput: `{ filePath, oldString,
//! newString, replaceAll? }`. Renders a Shiki-friendly diff fence in
//! `description` (per-language hl + `[!code ++/--]` markers when the
//! extension resolves; `\`\`\`diff` fallback otherwise).

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::{format_diff_hunk, pick, short_path, text_blocks};
use crate::tools::formatter::types::{FormattedToolCall, ToolField};

pub struct EditFormatter;

impl ToolFormatter for EditFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let path = pick::<String>(ctx.raw_input, "filePath").filter(|s| !s.is_empty());
        let replace_all = pick::<bool>(ctx.raw_input, "replaceAll").unwrap_or(false);
        let old_text = pick::<String>(ctx.raw_input, "oldString").unwrap_or_default();
        let new_text = pick::<String>(ctx.raw_input, "newString").unwrap_or_default();

        let title = match path.as_deref() {
            Some(p) => format!("edit · {}", short_path(p)),
            None => "edit".to_string(),
        };

        let description = format_diff_hunk(path.as_deref(), &old_text, &new_text);

        let mut fields: Vec<ToolField> = Vec::new();
        if let Some(p) = path {
            fields.push(ToolField {
                label: "path".into(),
                value: p,
            });
        }
        if replace_all {
            fields.push(ToolField {
                label: "replace all".into(),
                value: "true".into(),
            });
        }

        let block_text = text_blocks(ctx.content);
        let trimmed = block_text.trim();
        let output = if trimmed.is_empty() { None } else { Some(trimmed.to_string()) };

        FormattedToolCall {
            title,
            stat: None,
            description,
            output,
            fields,
        }
    }
}
