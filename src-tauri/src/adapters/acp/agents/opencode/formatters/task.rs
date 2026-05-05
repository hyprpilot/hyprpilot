//! opencode's `task` subagent tool. RawInput: `{ description, prompt,
//! subagent_type, task_id?, command? }`.

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::{pick, text_blocks};
use crate::tools::formatter::types::{FormattedToolCall, ToolField};

pub struct TaskFormatter;

impl ToolFormatter for TaskFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let description = pick::<String>(ctx.raw_input, "description").filter(|s| !s.is_empty());
        let subagent = pick::<String>(ctx.raw_input, "subagent_type").filter(|s| !s.is_empty());
        let prompt = pick::<String>(ctx.raw_input, "prompt").filter(|s| !s.is_empty());

        let title = match (description.as_deref(), subagent.as_deref()) {
            (Some(d), Some(s)) => format!("task · {} (@{})", d, s),
            (Some(d), None) => format!("task · {}", d),
            (None, Some(s)) => format!("task · @{}", s),
            (None, None) => "task".to_string(),
        };

        let body = prompt.map(|p| format!("```\n{}\n```", p));

        let mut fields: Vec<ToolField> = Vec::new();
        if let Some(s) = subagent {
            fields.push(ToolField {
                label: "subagent".into(),
                value: s,
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
            stat: None,
            description: body,
            output,
            fields,
        }
    }
}
