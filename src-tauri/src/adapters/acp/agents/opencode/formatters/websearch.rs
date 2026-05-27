//! opencode's `websearch` tool. RawInput: `{ query, numResults?,
//! livecrawl?, type?, contextMaxCharacters? }`.

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::{dedupe_output, duration_stats, pick};
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

        let stats = duration_stats(ctx);

        FormattedToolCall {
            title,
            stats,
            description: None,
            diff: None,
            output: dedupe_output(ctx.content, None),
            fields,
        }
    }
}
