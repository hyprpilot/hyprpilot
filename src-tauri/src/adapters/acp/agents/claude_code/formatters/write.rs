//! claude-code's `Write` tool. Creates / overwrites a file. Stat
//! surfaces the byte count of the new content.

use crate::tools::formatter::registry::{FormatterContext, FormatterRegistry, ToolFormatter};
use crate::tools::formatter::shared::{format_diff_hunk, line_magnitudes, pick, short_path, text_blocks};
use crate::tools::formatter::types::{FormattedToolCall, Stat};

pub struct WriteFormatter;

impl ToolFormatter for WriteFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let path = pick::<String>(ctx.raw_input, "file_path");
        let body = pick::<String>(ctx.raw_input, "content");
        let title = match path.as_deref() {
            Some(p) => format!("write · {}", short_path(p)),
            None => "write".to_string(),
        };

        // Diff-line stat: write replaces the file wholesale, so empty
        // old → all-add. The `+N` pill conveys both "this many lines"
        // and "all additions" at a glance, replacing the prior raw
        // char-count.
        let mut stats: Vec<Stat> = Vec::new();
        if let Some(new_text) = body.as_deref().filter(|s| !s.is_empty()) {
            let (added, removed) = line_magnitudes("", new_text);
            stats.push(Stat::Diff { added, removed });
        }

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
            stats,
            description,
            output,
            fields: Vec::new(),
        }
    }
}

pub fn register(reg: &mut FormatterRegistry, adapter: &str) {
    reg.register_adapter(adapter, "Write", Box::new(WriteFormatter));
}
