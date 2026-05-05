//! opencode's `write` tool. RawInput: `{ filePath, content }`.

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::{diff_line_counts, format_diff_hunk, pick, short_path};
use crate::tools::formatter::types::{FormattedToolCall, Stat, ToolField};

pub struct WriteFormatter;

impl ToolFormatter for WriteFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let path = pick::<String>(ctx.raw_input, "filePath").filter(|s| !s.is_empty());
        let body = pick::<String>(ctx.raw_input, "content");

        let title = match path.as_deref() {
            Some(p) => format!("write · {}", short_path(p)),
            None => "write".to_string(),
        };

        let mut stats: Vec<Stat> = Vec::new();
        if let Some(new_text) = body.as_deref().filter(|s| !s.is_empty()) {
            let (added, removed) = diff_line_counts("", new_text);
            stats.push(Stat::Diff { added, removed });
        }

        // Render the new content as a diff (all-add) so the captain
        // reviews the file before granting write permission. `content`
        // is consumed here; not dumped as a redundant field.
        let description = body
            .as_deref()
            .and_then(|new_text| format_diff_hunk(path.as_deref(), "", new_text));

        let mut fields: Vec<ToolField> = Vec::new();
        if let Some(p) = path {
            fields.push(ToolField {
                label: "path".into(),
                value: p,
            });
        }

        FormattedToolCall {
            title,
            stats,
            description,
            output: None,
            fields,
        }
    }
}
