//! codex-acp's plugin / MCP tool calls. Codex emits dynamic tools as
//! `Tool: <tool>` and MCP tools as `Tool: <server>/<leaf>` with
//! `rawInput` set to the serialized MCP invocation (`server`, `tool`,
//! `arguments`). Keep formatting adapter-local: higher layers only pass
//! the parsed identity when they have one.

use serde_json::Value;

use crate::adapters::ToolIdentity;
use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::{args_to_fields, duration_stats};
use crate::tools::formatter::types::FormattedToolCall;

pub struct ToolFormatterCodex;

struct McpParts<'a> {
    server: &'a str,
    leaf: &'a str,
    arguments: Option<&'a Value>,
}

impl ToolFormatter for ToolFormatterCodex {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let body = ctx.wire_name.strip_prefix("Tool: ").unwrap_or(ctx.wire_name);
        let mcp = mcp_parts(ctx.identity, ctx.raw_input);
        let title = match &mcp {
            Some(parts) => format!("mcp · {}/{}", parts.server, parts.leaf),
            None => format!("tool · {}", body),
        };

        let fields = match &mcp {
            Some(parts) => args_to_fields(parts.arguments, &[]),
            None => args_to_fields(ctx.raw_input, &[]),
        };

        let block_text = crate::tools::formatter::shared::text_blocks(ctx.content);
        let trimmed = block_text.trim();
        let output = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };

        let stats = duration_stats(ctx);

        FormattedToolCall {
            title,
            stats,
            description: None,
            diff: None,
            output,
            fields,
        }
    }
}

fn mcp_parts<'a>(identity: &'a ToolIdentity, raw_input: Option<&'a Value>) -> Option<McpParts<'a>> {
    let raw = raw_input;
    match identity {
        ToolIdentity::Mcp { server, leaf } => Some(McpParts {
            server,
            leaf,
            arguments: raw.and_then(|value| value.get("arguments")),
        }),
        ToolIdentity::Native => raw_mcp_parts(raw),
    }
}

fn raw_mcp_parts(raw_input: Option<&Value>) -> Option<McpParts<'_>> {
    let raw = raw_input?;
    let server = raw
        .get("server")
        .or_else(|| raw.get("server_name"))
        .or_else(|| raw.get("serverName"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())?;
    let leaf = raw
        .get("tool")
        .or_else(|| raw.get("tool_name"))
        .or_else(|| raw.get("toolName"))
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())?;

    Some(McpParts {
        server,
        leaf,
        arguments: raw.get("arguments"),
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::ToolFormatterCodex;
    use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};

    #[test]
    fn mcp_tool_formats_from_raw_invocation_without_duplicate_wrapper_fields() {
        let raw = json!({
            "server": "hyprpilot",
            "tool": "read_skill",
            "arguments": { "slug": "git-branch" }
        });
        let ctx = FormatterContext {
            wire_name: "Tool: hyprpilot/read_skill",
            identity: &crate::adapters::ToolIdentity::Native,
            kind: "other",
            raw_input: Some(&raw),
            adapter: "acp-codex",
            content: &[],
            started_at: 0,
            completed_at: None,
        };
        let formatted = ToolFormatterCodex.format(&ctx);

        assert_eq!(formatted.title, "mcp · hyprpilot/read_skill");
        assert!(formatted.fields.iter().all(|field| field.label != "server"));
        assert!(formatted.fields.iter().all(|field| field.label != "tool"));
        assert!(formatted.fields.iter().all(|field| field.label != "arguments"));
        assert!(formatted.fields.iter().any(|field| field.label == "slug"));
    }

    #[test]
    fn mcp_tool_accepts_codex_approval_style_names() {
        let raw = json!({
            "server_name": "memory",
            "tool_name": "read_graph",
            "arguments": {}
        });
        let ctx = FormatterContext {
            wire_name: "Tool: memory/read_graph",
            identity: &crate::adapters::ToolIdentity::Native,
            kind: "other",
            raw_input: Some(&raw),
            adapter: "acp-codex",
            content: &[],
            started_at: 0,
            completed_at: None,
        };
        let formatted = ToolFormatterCodex.format(&ctx);

        assert_eq!(formatted.title, "mcp · memory/read_graph");
        assert!(formatted.fields.is_empty());
    }
}
