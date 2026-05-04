use crate::tools::formatter::registry::{FormatterContext, FormatterRegistry, ToolFormatter};
use crate::tools::formatter::shared::{pick, short_path, text_blocks};
use crate::tools::formatter::types::FormattedToolCall;

pub struct GlobFormatter;

impl ToolFormatter for GlobFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let pattern = pick::<String>(ctx.raw_input, "pattern").filter(|s| !s.is_empty());
        let path = pick::<String>(ctx.raw_input, "path").unwrap_or_default();
        let trimmed = if path.is_empty() {
            String::new()
        } else {
            short_path(&path)
        };

        let title = match (pattern.as_deref(), trimmed.as_str()) {
            (Some(p), "") => format!("glob · {}", p),
            (Some(p), t) => format!("glob · {} in {}", p, t),
            (None, _) => "glob".to_string(),
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
    reg.register_adapter(adapter, "Glob", Box::new(GlobFormatter));
}
