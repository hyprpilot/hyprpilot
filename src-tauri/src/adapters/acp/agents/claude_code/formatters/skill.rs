use crate::tools::formatter::registry::{FormatterContext, FormatterRegistry, ToolFormatter};
use crate::tools::formatter::shared::{pick, text_blocks};
use crate::tools::formatter::types::FormattedToolCall;

pub struct SkillFormatter;

impl ToolFormatter for SkillFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let slug = pick::<String>(ctx.raw_input, "skill").filter(|s| !s.is_empty());
        let description = pick::<String>(ctx.raw_input, "description").filter(|s| !s.is_empty());
        let title = match slug {
            Some(s) => format!("skill · {}", s),
            None => "skill".to_string(),
        };
        let body = text_blocks(ctx.content);
        let output = if body.is_empty() { None } else { Some(body) };

        FormattedToolCall {
            title,
            stats: Vec::new(),
            description,
            output,
            fields: Vec::new(),
        }
    }
}

pub fn register(reg: &mut FormatterRegistry, adapter: &str) {
    reg.register_adapter(adapter, "Skill", Box::new(SkillFormatter));
}
