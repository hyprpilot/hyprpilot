//! `delete` — file / resource removal. RawInput convention is
//! `file_path` / `path`.

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::{args_to_fields, pick, wire_title_or_fallback};
use crate::tools::formatter::types::{FormattedToolCall, ToolField};

pub struct DeleteFormatter;

impl ToolFormatter for DeleteFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let path = pick::<String>(ctx.raw_input, "file_path")
            .or_else(|| pick(ctx.raw_input, "path"))
            .filter(|s| !s.is_empty());

        let title = wire_title_or_fallback(ctx.wire_name, "delete");

        let description = pick::<String>(ctx.raw_input, "description").filter(|s| !s.is_empty());

        let mut fields = args_to_fields(ctx.raw_input, &["file_path", "path", "description"]);
        if let Some(p) = path {
            fields.insert(
                0,
                ToolField {
                    label: "path".into(),
                    value: p,
                },
            );
        }

        FormattedToolCall {
            title,
            stat: None,
            description,
            output: None,
            fields,
        }
    }
}
