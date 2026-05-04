//! opencode's `grep` tool. RawInput: `{ pattern, path?, include? }`.

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::{pick, short_path, text_blocks};
use crate::tools::formatter::types::{FormattedToolCall, ToolField};

pub struct GrepFormatter;

impl ToolFormatter for GrepFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let pattern = pick::<String>(ctx.raw_input, "pattern").filter(|s| !s.is_empty());
        let path = pick::<String>(ctx.raw_input, "path").filter(|s| !s.is_empty());
        let include = pick::<String>(ctx.raw_input, "include").filter(|s| !s.is_empty());

        let title = match (pattern.as_deref(), path.as_deref()) {
            (Some(p), Some(root)) => format!("grep · {} in {}", p, short_path(root)),
            (Some(p), None) => format!("grep · {}", p),
            _ => "grep".to_string(),
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
        if let Some(i) = include {
            fields.push(ToolField {
                label: "include".into(),
                value: i,
            });
        }

        let block_text = text_blocks(ctx.content);
        let trimmed = block_text.trim();
        let output = if trimmed.is_empty() { None } else { Some(trimmed.to_string()) };

        FormattedToolCall {
            title,
            stat: None,
            description: None,
            output,
            fields,
        }
    }
}
