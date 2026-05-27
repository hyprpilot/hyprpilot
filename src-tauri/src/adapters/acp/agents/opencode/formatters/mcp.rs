//! opencode MCP tool family. opencode flattens MCP names as
//! `<server>_<tool>`; the adapter maps that back to `ToolKind::Mcp`
//! before dispatch reaches this formatter.

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::{args_to_fields, duration_stats, text_blocks};
use crate::tools::formatter::types::FormattedToolCall;

pub struct McpFormatter;

impl ToolFormatter for McpFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let title = match ctx.tool_kind {
            crate::tools::ToolKind::Mcp { server, tool } => format!("{server} · {tool}"),
            _ => format!("mcp · {}", ctx.wire_name),
        };

        let block_text = text_blocks(ctx.content);
        let trimmed = block_text.trim();
        let output = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };

        FormattedToolCall {
            title,
            stats: duration_stats(ctx),
            description: None,
            diff: None,
            output,
            fields: args_to_fields(ctx.raw_input, &[]),
        }
    }
}
