use crate::tools::formatter::registry::{FormatterContext, FormatterRegistry, ToolFormatter};
use crate::tools::formatter::shared::{pick, text_blocks};
use crate::tools::formatter::types::FormattedToolCall;

pub struct TaskFormatter;

impl ToolFormatter for TaskFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let subagent = pick::<String>(ctx.raw_input, "subagent_type")
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "agent".to_string());
        let description = pick::<String>(ctx.raw_input, "description").filter(|s| !s.is_empty());
        let prompt = pick::<String>(ctx.raw_input, "prompt").filter(|s| !s.is_empty());
        let title = match description {
            Some(d) => format!("task · {} — {}", subagent, d),
            None => format!("task · {}", subagent),
        };
        let body = prompt.map(|p| {
            if p.chars().count() > 200 {
                let mut s: String = p.chars().take(199).collect();
                s.push('…');
                s
            } else {
                p
            }
        });
        let block_text = text_blocks(ctx.content);
        let output = if block_text.is_empty() { None } else { Some(block_text) };

        FormattedToolCall {
            title,
            stats: Vec::new(),
            description: body,
            output,
            fields: Vec::new(),
        }
    }
}

pub fn register(reg: &mut FormatterRegistry, adapter: &str) {
    reg.register_adapter(adapter, "Task", Box::new(TaskFormatter));
}
