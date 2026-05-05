//! `read` — file reads. RawInput convention is `file_path`
//! (claude-code, codex) or `path` (opencode); `uri` for resource-style
//! reads.

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::{args_to_fields, dedupe_output, pick, wire_title_or_fallback};
use crate::tools::formatter::types::{FormattedToolCall, ToolField};

pub struct ReadFormatter;

impl ToolFormatter for ReadFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let path = pick::<String>(ctx.raw_input, "file_path")
            .or_else(|| pick(ctx.raw_input, "path"))
            .or_else(|| pick(ctx.raw_input, "uri"))
            .filter(|s| !s.is_empty());

        let title = wire_title_or_fallback(ctx.wire_name, "read");

        let description = pick::<String>(ctx.raw_input, "description").filter(|s| !s.is_empty());

        let mut fields = args_to_fields(ctx.raw_input, &["file_path", "path", "uri", "description"]);
        if let Some(p) = path {
            fields.insert(
                0,
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
