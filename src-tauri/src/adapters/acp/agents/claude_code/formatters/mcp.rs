//! claude-code's MCP tool family. Wire names are dynamic
//! (`mcp__<server>__<leaf>`), so the registry's `mcp__` prefix
//! exception routes every dynamic name to the single key
//! `(adapter, "mcp")` we register here.

use crate::tools::formatter::registry::{FormatterContext, FormatterRegistry, ToolFormatter};
use crate::tools::formatter::shared::{args_to_fields, parse_mcp, pick, text_blocks};
use crate::tools::formatter::types::FormattedToolCall;

pub struct McpFormatter;

impl ToolFormatter for McpFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let lower = ctx.wire_name.to_ascii_lowercase();
        let parsed = parse_mcp(&lower);
        let title = match &parsed {
            Some(p) => format!("{} · {}", p.server, p.leaf),
            None => format!("mcp · {}", ctx.wire_name),
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

        FormattedToolCall {
            title,
            stat: None,
            description,
            output,
            fields,
        }
    }
}

pub fn register(reg: &mut FormatterRegistry, adapter: &str) {
    reg.register_adapter(adapter, "mcp", Box::new(McpFormatter));
}
