//! `search` — file/content search. RawInput conventions:
//! `pattern` (claude-code Grep), `query` (codex), `q` (some MCP
//! tools). Optional `path` arg constrains the search root.

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::{args_to_fields, dedupe_output, pick, wire_title_or_fallback};
use crate::tools::formatter::types::{FormattedToolCall, ToolField};

pub struct SearchFormatter;

impl ToolFormatter for SearchFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let pattern = pick::<String>(ctx.raw_input, "pattern")
            .or_else(|| pick(ctx.raw_input, "query"))
            .or_else(|| pick(ctx.raw_input, "q"))
            .filter(|s| !s.is_empty());
        let path = pick::<String>(ctx.raw_input, "path").filter(|s| !s.is_empty());

        let title = wire_title_or_fallback(ctx.wire_name, "search");

        let description = pick::<String>(ctx.raw_input, "description").filter(|s| !s.is_empty());

        let mut fields = args_to_fields(ctx.raw_input, &["pattern", "query", "q", "path", "description"]);
        if let Some(p) = pattern {
            fields.insert(
                0,
                ToolField {
                    label: "pattern".into(),
                    value: p,
                },
            );
        }
        if let Some(p) = path {
            fields.insert(
                fields.len().min(1),
                ToolField {
                    label: "path".into(),
                    value: p,
                },
            );
        }

        let output = dedupe_output(ctx.content, description.as_deref());

        FormattedToolCall {
            title,
            stats: Vec::new(),
            description,
            output,
            fields,
        }
    }
}
