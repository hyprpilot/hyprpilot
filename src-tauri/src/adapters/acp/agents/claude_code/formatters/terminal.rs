use crate::tools::formatter::registry::{FormatterContext, FormatterRegistry, ToolFormatter};
use crate::tools::formatter::shared::{pick, text_blocks};
use crate::tools::formatter::types::FormattedToolCall;

pub struct TerminalFormatter;

impl ToolFormatter for TerminalFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let tid = pick::<String>(ctx.raw_input, "terminal_id");
        let command = pick::<String>(ctx.raw_input, "command").filter(|s| !s.is_empty());

        let title = match (command, tid) {
            (Some(c), Some(id)) => format!("terminal #{} · {}", id, c),
            (Some(c), None) => format!("terminal · {}", c),
            (None, Some(id)) => format!("terminal #{}", id),
            (None, None) => "terminal".to_string(),
        };
        let body = text_blocks(ctx.content);
        let output = if body.is_empty() { None } else { Some(body) };

        FormattedToolCall {
            title,
            stat: None,
            description: None,
            output,
            fields: Vec::new(),
        }
    }
}

pub fn register(reg: &mut FormatterRegistry, adapter: &str) {
    reg.register_adapter(adapter, "Terminal", Box::new(TerminalFormatter));
}
