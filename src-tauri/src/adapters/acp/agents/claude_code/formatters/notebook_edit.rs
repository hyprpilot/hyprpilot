use crate::tools::formatter::registry::{FormatterContext, FormatterRegistry, ToolFormatter};
use crate::tools::formatter::shared::{pick, short_path, text_blocks};
use crate::tools::formatter::types::FormattedToolCall;

pub struct NotebookEditFormatter;

impl ToolFormatter for NotebookEditFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let path = pick::<String>(ctx.raw_input, "notebook_path");
        let cell_id = pick::<String>(ctx.raw_input, "cell_id").filter(|s| !s.is_empty());
        let edit_mode = pick::<String>(ctx.raw_input, "edit_mode").filter(|s| !s.is_empty());

        let mut bits: Vec<String> = Vec::new();
        if let Some(c) = cell_id {
            bits.push(format!("cell {}", c));
        }
        if let Some(m) = edit_mode {
            bits.push(m);
        }
        let suffix = if bits.is_empty() {
            String::new()
        } else {
            format!(" ({})", bits.join(" · "))
        };
        let title = match path {
            Some(p) => format!("notebook · {}{}", short_path(&p), suffix),
            None => format!("notebook{}", suffix),
        };

        let output_text = text_blocks(ctx.content);
        let output = if output_text.is_empty() {
            None
        } else {
            Some(output_text)
        };

        FormattedToolCall {
            title,
            stats: Vec::new(),
            description: None,
            output,
            fields: Vec::new(),
        }
    }
}

pub fn register(reg: &mut FormatterRegistry, adapter: &str) {
    reg.register_adapter(adapter, "NotebookEdit", Box::new(NotebookEditFormatter));
}
