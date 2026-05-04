//! opencode's `skill` tool. RawInput: `{ name }`. Resolves a named
//! skill from opencode's registry; output is the skill body.

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::{pick, text_blocks};
use crate::tools::formatter::types::{FormattedToolCall, ToolField};

pub struct SkillFormatter;

impl ToolFormatter for SkillFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let name = pick::<String>(ctx.raw_input, "name").filter(|s| !s.is_empty());

        let title = match name.as_deref() {
            Some(n) => format!("skill · {}", n),
            None => "skill".to_string(),
        };

        let mut fields: Vec<ToolField> = Vec::new();
        if let Some(n) = name {
            fields.push(ToolField {
                label: "name".into(),
                value: n,
            });
        }

        let block_text = text_blocks(ctx.content);
        let trimmed = block_text.trim();
        let description = if trimmed.is_empty() { None } else { Some(trimmed.to_string()) };

        FormattedToolCall {
            title,
            stat: None,
            description,
            output: None,
            fields,
        }
    }
}
