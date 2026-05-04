use crate::tools::formatter::registry::{FormatterContext, FormatterRegistry, ToolFormatter};
use crate::tools::formatter::shared::{pick, text_blocks};
use crate::tools::formatter::types::FormattedToolCall;

pub struct WebSearchFormatter;

impl ToolFormatter for WebSearchFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let query = pick::<String>(ctx.raw_input, "query").filter(|s| !s.is_empty());
        let allowed = pick::<Vec<String>>(ctx.raw_input, "allowed_domains").unwrap_or_default();
        let blocked = pick::<Vec<String>>(ctx.raw_input, "blocked_domains").unwrap_or_default();

        let mut bits: Vec<String> = Vec::new();
        if !allowed.is_empty() {
            bits.push(format!("allowed: {}", allowed.join(", ")));
        }
        if !blocked.is_empty() {
            bits.push(format!("blocked: {}", blocked.join(", ")));
        }
        let suffix = if bits.is_empty() {
            String::new()
        } else {
            format!(" ({})", bits.join(" · "))
        };
        let title = match query {
            Some(q) => format!("search · {}{}", q, suffix),
            None => "search".to_string(),
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
    reg.register_adapter(adapter, "WebSearch", Box::new(WebSearchFormatter));
}
