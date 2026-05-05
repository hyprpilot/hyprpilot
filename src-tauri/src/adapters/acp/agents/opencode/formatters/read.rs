//! opencode's `read` tool. RawInput: `{ filePath, offset?, limit? }`.

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::{pick, short_path, text_blocks};
use crate::tools::formatter::types::{FormattedToolCall, ToolField};

pub struct ReadFormatter;

impl ToolFormatter for ReadFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let path = pick::<String>(ctx.raw_input, "filePath").filter(|s| !s.is_empty());
        let offset = pick::<i64>(ctx.raw_input, "offset");
        let limit = pick::<i64>(ctx.raw_input, "limit");

        let title = match path.as_deref() {
            Some(p) => format!("read · {}", short_path(p)),
            None => "read".to_string(),
        };

        let mut fields: Vec<ToolField> = Vec::new();
        if let Some(p) = path {
            fields.push(ToolField {
                label: "path".into(),
                value: p,
            });
        }
        if let Some(o) = offset {
            fields.push(ToolField {
                label: "offset".into(),
                value: o.to_string(),
            });
        }
        if let Some(l) = limit {
            fields.push(ToolField {
                label: "limit".into(),
                value: l.to_string(),
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
