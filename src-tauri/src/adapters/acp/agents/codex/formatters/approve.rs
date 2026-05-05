//! codex-acp's MCP-tool approval elicitation. Title shape:
//! `Approve <tool_title>` (when meta carries the tool name) or
//! `Approve MCP tool call` (fallback). RawInput is the elicitation
//! request payload — passed through as fields.

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::{args_to_fields, text_blocks};
use crate::tools::formatter::types::FormattedToolCall;

pub struct ApproveFormatter;

impl ToolFormatter for ApproveFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let title = if ctx.wire_name.trim().is_empty() {
            "approve".to_string()
        } else {
            ctx.wire_name.trim().to_string()
        };

        let block_text = text_blocks(ctx.content);
        let trimmed = block_text.trim();
        let description = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };

        let fields = args_to_fields(ctx.raw_input, &[]);

        FormattedToolCall {
            title,
            stats: Vec::new(),
            description,
            output: None,
            fields,
        }
    }
}
