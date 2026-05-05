use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::{pick, text_blocks};
use crate::tools::formatter::types::FormattedToolCall;

/// `think` — agent's internal reasoning. The only kind whose args
/// shape is somewhat standardised across vendors (`thought` / `text`
/// is the convention claude-code, codex, opencode all follow). Pull
/// the body into `description` so it renders as markdown.
pub struct ThinkFormatter;

impl ToolFormatter for ThinkFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let body = pick::<String>(ctx.raw_input, "thought").or_else(|| pick(ctx.raw_input, "text"));
        let blocks = text_blocks(ctx.content);
        let description = body.or(if blocks.is_empty() { None } else { Some(blocks) });

        let title = if ctx.wire_name.trim().is_empty() {
            "thinking".to_string()
        } else {
            ctx.wire_name.trim().to_string()
        };

        FormattedToolCall {
            title,
            stats: Vec::new(),
            description,
            output: None,
            fields: Vec::new(),
        }
    }
}
