//! opencode's `websearch` tool. RawInput: `{ query, numResults?,
//! livecrawl?, type?, contextMaxCharacters? }`.

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::{pick, text_blocks};
use crate::tools::formatter::types::{FormattedToolCall, ToolField};

pub struct WebSearchFormatter;

impl ToolFormatter for WebSearchFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let query = pick::<String>(ctx.raw_input, "query").filter(|s| !s.is_empty());
        let num = pick::<i64>(ctx.raw_input, "numResults");

        let title = match query.as_deref() {
            Some(q) => format!("websearch · {}", q),
            None => "websearch".to_string(),
        };

        let mut fields: Vec<ToolField> = Vec::new();
        if let Some(q) = query {
            fields.push(ToolField {
                label: "query".into(),
                value: q,
            });
        }
        if let Some(n) = num {
            fields.push(ToolField {
                label: "num results".into(),
                value: n.to_string(),
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
            description: None,
            output,
            fields,
        }
    }
}
