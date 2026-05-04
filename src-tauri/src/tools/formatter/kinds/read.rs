//! `read` — file reads. RawInput convention is `file_path`
//! (claude-code, codex) or `path` (opencode); `uri` for resource-style
//! reads.

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::{args_to_fields, pick, short_path, dedupe_output, title_prefix};
use crate::tools::formatter::types::{FormattedToolCall, ToolField};

pub struct ReadFormatter;

impl ToolFormatter for ReadFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let path = pick::<String>(ctx.raw_input, "file_path")
            .or_else(|| pick(ctx.raw_input, "path"))
            .or_else(|| pick(ctx.raw_input, "uri"))
            .filter(|s| !s.is_empty());

        let prefix = title_prefix(ctx.wire_name, "read");
        let title = match path.as_deref() {
            Some(p) => format!("{} · {}", prefix, short_path(p)),
            None => prefix,
        };

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
            stat: None,
            description,
            output,
            fields,
        }
    }
}
