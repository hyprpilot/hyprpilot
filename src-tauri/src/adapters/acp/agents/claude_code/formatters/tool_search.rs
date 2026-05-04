use crate::tools::formatter::registry::{FormatterContext, FormatterRegistry, ToolFormatter};
use crate::tools::formatter::shared::{pick, text_blocks};
use crate::tools::formatter::types::FormattedToolCall;

pub struct ToolSearchFormatter;

impl ToolFormatter for ToolSearchFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let query = pick::<String>(ctx.raw_input, "query").filter(|s| !s.is_empty());
        let max = pick::<i64>(ctx.raw_input, "max_results");
        let suffix = match max {
            Some(m) if m > 0 && m != 5 => format!(" (max {})", m),
            _ => String::new(),
        };
        let title = match query {
            Some(q) => format!("tool search · {}{}", q, suffix),
            None => "tool search".to_string(),
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
    reg.register_adapter(adapter, "ToolSearch", Box::new(ToolSearchFormatter));
}
