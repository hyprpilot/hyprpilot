//! opencode's `edit` tool. RawInput: `{ filePath, oldString,
//! newString, replaceAll? }`. Renders a Shiki-friendly diff fence in
//! `description` (per-language hl + `[!code ++/--]` markers when the
//! extension resolves; `\`\`\`diff` fallback otherwise).

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::{dedupe_output, format_diff_hunk, format_git_diff, pick};
use crate::tools::formatter::types::{FormattedToolCall, ToolField};

pub struct EditFormatter;

impl ToolFormatter for EditFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let path = pick::<String>(ctx.raw_input, "filePath")
            .or_else(|| pick::<String>(ctx.raw_input, "filepath"))
            .filter(|s| !s.is_empty());
        let replace_all = pick::<bool>(ctx.raw_input, "replaceAll").unwrap_or(false);
        let old_text = pick::<String>(ctx.raw_input, "oldString").unwrap_or_default();
        let new_text = pick::<String>(ctx.raw_input, "newString").unwrap_or_default();
        let metadata_diff = pick::<String>(ctx.raw_input, "diff").filter(|s| !s.is_empty());

        let title = match path.as_deref() {
            Some(p) => format!("edit · {}", p),
            None => "edit".to_string(),
        };

        let description = metadata_diff
            .as_ref()
            .map(|diff| format!("```diff\n{diff}\n```"))
            .or_else(|| format_diff_hunk(path.as_deref(), &old_text, &new_text));
        let diff = metadata_diff
            .clone()
            .or_else(|| format_git_diff(path.as_deref(), &old_text, &new_text));

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

        FormattedToolCall {
            title,
            stats: Vec::new(),
            description,
            diff,
            output: dedupe_output(ctx.content, None),
            fields,
        }
    }
}
