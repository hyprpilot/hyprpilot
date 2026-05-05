//! opencode's `glob` tool. RawInput: `{ pattern, path? }`.

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::{pick, short_path, text_blocks};
use crate::tools::formatter::types::{FormattedToolCall, ToolField};

pub struct GlobFormatter;

impl ToolFormatter for GlobFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let pattern = pick::<String>(ctx.raw_input, "pattern").filter(|s| !s.is_empty());
        let path = pick::<String>(ctx.raw_input, "path").filter(|s| !s.is_empty());

        let title = match (pattern.as_deref(), path.as_deref()) {
            (Some(p), Some(root)) => format!("glob · {} in {}", p, short_path(root)),
            (Some(p), None) => format!("glob · {}", p),
            _ => "glob".to_string(),
        };

        let mut fields: Vec<ToolField> = Vec::new();
        if let Some(p) = pattern {
            fields.push(ToolField {
                label: "pattern".into(),
                value: p,
            });
        }
        if let Some(p) = path {
            fields.push(ToolField {
                label: "path".into(),
                value: p,
            });
        }

        let block_text = text_blocks(ctx.content);
        let trimmed = block_text.trim();
        let output = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };

        FormattedToolCall {
            title,
            stats: Vec::new(),
            description: None,
            output,
            fields,
        }
    }
}
