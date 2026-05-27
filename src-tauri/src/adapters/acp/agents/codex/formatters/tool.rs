//! codex-acp's plugin / MCP tool calls. Codex emits dynamic tools as
//! `Tool: <tool>` and MCP tools as `Tool: <server>/<leaf>` with
//! `rawInput` set to the serialized MCP invocation (`server`, `tool`,
//! `arguments`). Keep formatting adapter-local: higher layers only pass
//! MCP attribution when they have it.

use serde_json::Value;

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::{args_to_fields, dedupe_output, duration_stats};
use crate::tools::formatter::types::FormattedToolCall;

pub struct ToolFormatterCodex;

struct McpParts<'a> {
    server: &'a str,
    tool: &'a str,
    arguments: Option<&'a Value>,
}

impl ToolFormatter for ToolFormatterCodex {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let body = ctx.wire_name.strip_prefix("Tool: ").unwrap_or(ctx.wire_name);
        let mcp = mcp_parts(ctx);
        if mcp.is_none() && super::exec::matches(ctx) {
            return super::exec::ExecFormatter.format(ctx);
        }

        let title = match &mcp {
            Some(parts) => format!("mcp · {}/{}", parts.server, parts.tool),
            None => format!("tool · {}", body),
        };

        let fields = match &mcp {
            Some(parts) => args_to_fields(parts.arguments, &[]),
            None => args_to_fields(ctx.raw_input, &[]),
        };

        let output = dedupe_output(ctx.content, None);

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

fn mcp_parts<'a>(ctx: &'a FormatterContext<'a>) -> Option<McpParts<'a>> {
    let raw = ctx.raw_input;
    match ctx.tool_kind {
        crate::tools::ToolKind::Mcp { server, tool } => Some(McpParts {
            server,
            tool,
            arguments: raw.and_then(|value| value.get("arguments")),
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::ToolFormatterCodex;
    use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
    use crate::tools::ToolKind;

    #[test]
    fn mcp_tool_formats_from_raw_invocation_without_duplicate_wrapper_fields() {
        let raw = json!({
            "server": "hyprpilot",
            "tool": "read_skill",
            "arguments": { "slug": "git-branch" }
        });
        let mcp = ToolKind::Mcp {
            server: "hyprpilot".into(),
            tool: "read_skill".into(),
        };
        let ctx = FormatterContext {
            wire_name: "Tool: hyprpilot/read_skill",
            tool_kind: &mcp,
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
        let mcp = ToolKind::Mcp {
            server: "memory".into(),
            tool: "read_graph".into(),
        };
        let ctx = FormatterContext {
            wire_name: "Tool: memory/read_graph",
            tool_kind: &mcp,
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

    #[test]
    fn tool_wrapped_exec_command_uses_exec_formatter() {
        let raw = json!({
            "cmd": "git status --short",
            "workdir": "/repo"
        });
        let ctx = FormatterContext {
            wire_name: "Tool: exec_command",
            tool_kind: &crate::tools::ToolKind::Other,
            raw_input: Some(&raw),
            adapter: "acp-codex",
            content: &[],
            started_at: 0,
            completed_at: None,
        };
        let formatted = ToolFormatterCodex.format(&ctx);

        assert_eq!(formatted.title, "exec_command · git status --short");
        assert!(formatted.fields.iter().any(|field| field.label == "command"));
        assert!(formatted.fields.iter().any(|field| field.value == "git status --short"));
    }
}
