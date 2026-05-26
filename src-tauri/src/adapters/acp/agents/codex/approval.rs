use serde_json::Value;

use crate::adapters::ToolIdentity;
use crate::tools::formatter::shared::text_blocks;
use crate::tools::formatter::types::ToolField;

pub struct McpApproval {
    pub server: String,
    pub tool: String,
    pub description: Option<String>,
    pub fields: Vec<ToolField>,
}

impl McpApproval {
    pub fn title(&self) -> String {
        format!("Approve {}/{}", self.server, self.tool)
    }

    pub fn identity(&self) -> ToolIdentity {
        ToolIdentity::Mcp {
            server: self.server.clone(),
            leaf: self.tool.clone(),
        }
    }
}

pub fn parse_mcp(raw: Option<&Value>, content: &[Value]) -> Option<McpApproval> {
    let raw = raw?;
    let request = raw.get("request").unwrap_or(raw);
    let meta = approval_meta(request).or_else(|| approval_meta(raw))?;
    let message = request
        .get("message")
        .or_else(|| raw.get("message"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let (message_server, message_tool) = parse_message(message).unwrap_or_default();
    let tool_title = meta
        .get("tool_title")
        .or_else(|| request.get("tool_title"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let (title_server, title_tool) = tool_title.split_once('/').unwrap_or(("", tool_title));
    let server = raw
        .get("server_name")
        .or_else(|| raw.get("serverName"))
        .and_then(Value::as_str)
        .filter(|server| !server.trim().is_empty())
        .or_else(|| (!title_server.trim().is_empty()).then_some(title_server))
        .or_else(|| (!message_server.trim().is_empty()).then_some(message_server.as_str()))?
        .to_string();
    let tool = (!title_tool.trim().is_empty())
        .then_some(title_tool)
        .or_else(|| (!message_tool.trim().is_empty()).then_some(message_tool.as_str()))?
        .to_string();
    let content_text = text_blocks(content);
    let description = if content_text.trim().is_empty() {
        description_from_payload(meta, request, message)
    } else {
        Some(content_text.trim().to_string())
    };
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
    fields.extend(display_fields(meta, request));

    Some(McpApproval {
        server,
        tool,
        description,
        fields,
    })
}

fn approval_meta(value: &Value) -> Option<&serde_json::Map<String, Value>> {
    let meta = value
        .get("_meta")
        .or_else(|| value.get("meta"))
        .and_then(Value::as_object)?;
    (meta.get("codex_approval_kind").and_then(Value::as_str) == Some("mcp_tool_call")).then_some(meta)
}

fn parse_message(message: &str) -> Option<(String, String)> {
    let server = message.strip_prefix("Allow the ")?.split_once(" MCP server")?.0.trim();
    let tool = message.split_once("run tool \"")?.1.split_once('"')?.0.trim();

    if server.is_empty() || tool.is_empty() {
        return None;
    }

    Some((server.to_string(), tool.to_string()))
}

fn description_from_payload(meta: &serde_json::Map<String, Value>, request: &Value, message: &str) -> Option<String> {
    let mut parts = Vec::new();
    if !message.trim().is_empty() {
        parts.push(message.trim().to_string());
    }
    for key in ["connector_description", "tool_description"] {
        if let Some(description) = meta
            .get(key)
            .or_else(|| request.get(key))
            .and_then(Value::as_str)
            .filter(|description| !description.trim().is_empty())
        {
            parts.push(description.to_string());
        }
    }

    (!parts.is_empty()).then(|| parts.join("\n\n"))
}

fn display_fields(meta: &serde_json::Map<String, Value>, request: &Value) -> Vec<ToolField> {
    let mut fields: Vec<ToolField> = meta
        .get("tool_params_display")
        .or_else(|| request.get("tool_params_display"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(display_field)
        .collect();

    if fields.is_empty() {
        if let Some(params) = meta
            .get("tool_params")
            .or_else(|| request.get("tool_params"))
            .and_then(display_value)
        {
            fields.push(ToolField {
                label: "arguments".into(),
                value: params,
            });
        }
    }

    fields
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
