//! codex-acp's approval elicitation formatter. MCP tool approval
//! prompts carry their real server/tool identity inside rawInput
//! metadata, while the visible ACP title is the generic
//! `Approve MCP tool call`.

use serde_json::Value;

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::{args_to_fields, text_blocks};
use crate::tools::formatter::types::{FormattedToolCall, ToolField};

pub struct ApproveFormatter;

impl ToolFormatter for ApproveFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        if let Some(approval) = parse_mcp_approval(ctx.raw_input) {
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

struct McpApproval {
    server: String,
    tool: String,
    description: Option<String>,
    fields: Vec<ToolField>,
}

impl McpApproval {
    fn title(&self) -> String {
        format!("Approve {}/{}", self.server, self.tool)
    }
}

fn parse_mcp_approval(raw: Option<&Value>) -> Option<McpApproval> {
    let payload = approval_payload(raw?)?;
    let message = payload.get("message").and_then(Value::as_str).unwrap_or_default();
    let (server, tool) = parse_message(message)?;
    let description = payload
        .get("tool_description")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .map(str::to_string)
        .or_else(|| (!message.trim().is_empty()).then(|| message.to_string()));
    let mut fields = vec![
        ToolField {
            label: "server".into(),
            value: server.clone(),
        },
        ToolField {
            label: "tool".into(),
            value: tool.clone(),
        },
    ];
    fields.extend(display_fields(payload));

    Some(McpApproval {
        server,
        tool,
        description,
        fields,
    })
}

fn approval_payload(raw: &Value) -> Option<&Value> {
    if is_mcp_approval(raw) {
        return Some(raw);
    }
    raw.get("request").filter(|request| is_mcp_approval(request))
}

fn is_mcp_approval(value: &Value) -> bool {
    value
        .get("_meta")
        .or_else(|| value.get("meta"))
        .and_then(|meta| meta.get("codex_approval_kind"))
        .and_then(Value::as_str)
        == Some("mcp_tool_call")
}

fn parse_message(message: &str) -> Option<(String, String)> {
    let server = message.strip_prefix("Allow the ")?.split_once(" MCP server")?.0.trim();
    let tool = message.split_once("run tool \"")?.1.split_once('"')?.0.trim();

    if server.is_empty() || tool.is_empty() {
        return None;
    }

    Some((server.to_string(), tool.to_string()))
}

fn display_fields(payload: &Value) -> Vec<ToolField> {
    payload
        .get("tool_params_display")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(display_field)
        .collect()
}

fn display_field(value: &Value) -> Option<ToolField> {
    let obj = value.as_object()?;
    let label = obj
        .get("display_name")
        .or_else(|| obj.get("name"))
        .and_then(Value::as_str)?
        .trim();
    if label.is_empty() {
        return None;
    }

    let value = display_value(obj.get("value")?)?;

    Some(ToolField {
        label: label.to_string(),
        value,
    })
}

fn display_value(value: &Value) -> Option<String> {
    match value {
        Value::String(value) if !value.is_empty() => Some(value.clone()),
        Value::String(_) | Value::Null => None,
        Value::Bool(value) => Some(value.to_string()),
        Value::Number(value) => Some(value.to_string()),
        value => serde_json::to_string(value).ok(),
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
                "_meta": { "codex_approval_kind": "mcp_tool_call" },
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
            kind: "other",
            raw_input: Some(&raw),
            adapter: "acp-codex",
            content: &[],
            started_at: 0,
            completed_at: None,
        };
        let formatted = ApproveFormatter.format(&ctx);

        assert_eq!(formatted.title, "Approve hyprpilot/read_skill");
        assert_eq!(
            formatted.description.as_deref(),
            Some("Read a skill's full SKILL.md body.")
        );
        assert!(formatted
            .fields
            .iter()
            .any(|field| field.label == "slug" && field.value == "git-branch"));
        assert!(!formatted.fields.iter().any(|field| field.label == "request"));
    }
}
