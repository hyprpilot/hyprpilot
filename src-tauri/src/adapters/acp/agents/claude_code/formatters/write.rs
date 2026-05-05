//! claude-code's `Write` tool. Creates / overwrites a file. Stat
//! surfaces the byte count of the new content.

use crate::tools::formatter::registry::{FormatterContext, FormatterRegistry, ToolFormatter};
use crate::tools::formatter::shared::{format_diff_hunk, pick, short_path, text_blocks};
use crate::tools::formatter::types::FormattedToolCall;

pub struct WriteFormatter;

impl ToolFormatter for WriteFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let path = pick::<String>(ctx.raw_input, "file_path");
        let body = pick::<String>(ctx.raw_input, "content");
        let title = match path.as_deref() {
            Some(p) => format!("write · {}", short_path(p)),
            None => "write".to_string(),
        };
        let stat = body
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| format!("{} chars", s.len()));

        // Render the new content as a diff (empty old → all-add) so
        // the captain reviews the file before granting write
        // permission. Same per-language Shiki rendering as Edit;
        // `content` is consumed here, not dumped as a redundant field.
        let description = body
            .as_deref()
            .and_then(|new_text| format_diff_hunk(path.as_deref(), "", new_text));

        let output_text = text_blocks(ctx.content);
        let output = if output_text.is_empty() {
            None
        } else {
            Some(output_text)
        };

        FormattedToolCall {
            title,
            stat,
            description,
            output,
            fields: Vec::new(),
        }
    }
}

pub fn register(reg: &mut FormatterRegistry, adapter: &str) {
    reg.register_adapter(adapter, "Write", Box::new(WriteFormatter));
}
