//! codex-acp's plugin / MCP tool calls. Title shape is
//! `Tool: <tool>` (dynamic plugin) or `Tool: <server>/<leaf>` (MCP).
//! RawInput is whatever the agent passed as `arguments` (free-form
//! JSON for plugin tools; `McpInvocation { server, tool, arguments }`
//! for MCP). We pass the title through and dump rawInput as a
//! single field for visibility — per-server overrides land later.

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::args_to_fields;
use crate::tools::formatter::types::{FormattedToolCall, ToolField};

pub struct ToolFormatterCodex;

impl ToolFormatter for ToolFormatterCodex {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let body = ctx.wire_name.strip_prefix("Tool: ").unwrap_or(ctx.wire_name);
        let title = if body.contains('/') {
            format!("mcp · {}", body)
        } else {
            format!("tool · {}", body)
        };

        let mut fields: Vec<ToolField> = Vec::new();
        if let Some((server, leaf)) = body.split_once('/') {
            fields.push(ToolField {
                label: "server".into(),
                value: server.to_string(),
            });
            fields.push(ToolField {
                label: "tool".into(),
                value: leaf.to_string(),
            });
        }
        fields.extend(args_to_fields(ctx.raw_input, &[]));

        let block_text = crate::tools::formatter::shared::text_blocks(ctx.content);
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
