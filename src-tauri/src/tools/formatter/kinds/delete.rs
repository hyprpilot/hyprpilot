//! `delete` — file / resource removal. RawInput convention is
//! `file_path` / `path`.

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::{args_to_fields, pick, short_path, title_prefix};
use crate::tools::formatter::types::{FormattedToolCall, ToolField};

pub struct DeleteFormatter;

impl ToolFormatter for DeleteFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let path = pick::<String>(ctx.raw_input, "file_path")
            .or_else(|| pick(ctx.raw_input, "path"))
            .filter(|s| !s.is_empty());

        let prefix = title_prefix(ctx.wire_name, "delete");
        let title = match path.as_deref() {
            Some(p) => format!("{} · {}", prefix, short_path(p)),
            None => prefix,
        };

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
