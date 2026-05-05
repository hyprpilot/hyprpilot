//! claude-code's `Read` tool. file_path / offset / limit are the
//! standard claude-code args; body lands in `description` as a fenced
//! block keyed off the path's language.

use crate::tools::formatter::registry::{FormatterContext, FormatterRegistry, ToolFormatter};
use crate::tools::formatter::shared::{lang_from_path, pick, short_path, text_blocks};
use crate::tools::formatter::types::FormattedToolCall;

pub struct ReadFormatter;

impl ToolFormatter for ReadFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let path = pick::<String>(ctx.raw_input, "file_path").unwrap_or_default();
        let offset = pick::<i64>(ctx.raw_input, "offset");
        let limit = pick::<i64>(ctx.raw_input, "limit");

        let trimmed = if path.is_empty() {
            String::new()
        } else {
            short_path(&path)
        };

        let title = if !trimmed.is_empty() {
            match (offset, limit) {
                (Some(o), Some(l)) => format!("read · {} (lines {}..{})", trimmed, o, o + l),
                (Some(o), None) => format!("read · {} (from {})", trimmed, o),
                _ => format!("read · {}", trimmed),
            }
        } else {
            "read".to_string()
        };

        let body = text_blocks(ctx.content);
        let description = if !body.is_empty() {
            let lang = lang_from_path(&path).unwrap_or("plaintext");
            Some(format!("```{}\n{}\n```", lang, body))
        } else {
            None
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

pub fn register(reg: &mut FormatterRegistry, adapter: &str) {
    reg.register_adapter(adapter, "Read", Box::new(ReadFormatter));
}
