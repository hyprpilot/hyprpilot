//! codex-acp's approval elicitation formatter. MCP tool approval
//! prompts carry their real server/tool identity inside rawInput
//! metadata, while the visible ACP title is the generic
//! `Approve MCP tool call`.

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::{args_to_fields, text_blocks};
use crate::tools::formatter::types::FormattedToolCall;

use super::super::approval;

pub struct ApproveFormatter;

impl ToolFormatter for ApproveFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        if let Some(approval) = approval::parse_mcp(ctx.raw_input, ctx.content) {
            return FormattedToolCall {
                title: approval.title(),
                stats: Vec::new(),
                description: approval.description,
                diff: None,
                output: None,
                fields: approval.fields,
            };
        }

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
            diff: None,
            output: None,
            fields,
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::ApproveFormatter;
    use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};

    #[test]
    fn mcp_tool_approval_uses_metadata_instead_of_raw_request_dump() {
        let raw = json!({
            "request": {
                "meta": { "codex_approval_kind": "mcp_tool_call" },
                "message": "Allow the hyprpilot MCP server to run tool \"read_skill\"?",
                "tool_description": "Read a skill's full SKILL.md body.",
                "tool_params_display": [
                    { "display_name": "slug", "name": "slug", "value": "git-branch" }
                ],
                "requested_schema": { "type": "object" }
            }
        });
        let ctx = FormatterContext {
            wire_name: "Approve MCP tool call",
            identity: &crate::adapters::ToolIdentity::Native,
            kind: "other",
            raw_input: Some(&raw),
            adapter: "acp-codex",
            content: &[],
            started_at: 0,
            completed_at: None,
        };
        let formatted = ApproveFormatter.format(&ctx);

        assert_eq!(formatted.title, "Approve hyprpilot/read_skill");
        assert!(formatted
            .description
            .as_deref()
            .is_some_and(|description| description.contains("Read a skill's full SKILL.md body.")));
        assert!(formatted
            .fields
            .iter()
            .any(|field| field.label == "slug" && field.value == "git-branch"));
        assert!(!formatted.fields.iter().any(|field| field.label == "request"));
    }

    #[test]
    fn mcp_tool_approval_uses_upstream_elicitation_shape() {
        let raw = json!({
            "server_name": "hyprpilot",
            "id": "mcp_tool_call_approval_123",
            "request": {
                "meta": {
                    "codex_approval_kind": "mcp_tool_call",
                    "tool_title": "hyprpilot/read_skill",
                    "tool_description": "Read a skill's full SKILL.md body.",
                    "tool_params": { "slug": "git-branch" }
                },
                "message": "Allow the hyprpilot MCP server to run tool \"read_skill\"?"
            }
        });
        let ctx = FormatterContext {
            wire_name: "Approve hyprpilot/read_skill",
            identity: &crate::adapters::ToolIdentity::Native,
            kind: "other",
            raw_input: Some(&raw),
            adapter: "acp-codex",
            content: &[],
            started_at: 0,
            completed_at: None,
        };
        let formatted = ApproveFormatter.format(&ctx);

        assert_eq!(formatted.title, "Approve hyprpilot/read_skill");
        assert!(formatted
            .fields
            .iter()
            .any(|field| field.label == "arguments" && field.value.contains("git-branch")));
    }
}
