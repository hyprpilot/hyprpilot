//! claude-code's MCP tool family. The formatter registry routes the
//! structured MCP identity to the single `(adapter, "mcp")` key we
//! register here.

use crate::tools::formatter::registry::{FormatterContext, FormatterRegistry, ToolFormatter};
use crate::tools::formatter::shared::{args_to_fields, duration_stats, pick, text_blocks};
use crate::tools::formatter::types::FormattedToolCall;

pub struct McpFormatter;

impl ToolFormatter for McpFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let title = match ctx.identity {
            crate::adapters::ToolIdentity::Mcp { server, leaf } => format!("{server} · {leaf}"),
            crate::adapters::ToolIdentity::Native => format!("mcp · {}", ctx.wire_name),
        };

        let description = pick::<String>(ctx.raw_input, "description").filter(|s| !s.is_empty());
        let fields = args_to_fields(ctx.raw_input, &["description"]);

        let block_text = text_blocks(ctx.content);
        let trimmed = block_text.trim();
        let output = if !trimmed.is_empty() && trimmed != description.as_deref().unwrap_or("").trim() {
            Some(trimmed.to_string())
        } else {
            None
        };

        let stats = duration_stats(ctx);

        FormattedToolCall {
            title,
            stats,
            description,
            diff: None,
            output,
            fields,
        }
    }
}

pub fn register(reg: &mut FormatterRegistry, adapter: &str) {
    reg.register_adapter(adapter, "mcp", Box::new(McpFormatter));
}
