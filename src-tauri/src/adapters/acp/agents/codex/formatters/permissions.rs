//! codex-acp's broad permission request tool. Title is either the
//! supplied reason or `Permissions Request`; rawInput carries
//! `RequestPermissionsEvent`, and content carries a preformatted
//! summary of requested filesystem / network access.

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::{pick, text_blocks};
use crate::tools::formatter::types::{FormattedToolCall, ToolField};

pub struct PermissionsFormatter;

impl ToolFormatter for PermissionsFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let title = if ctx.wire_name.trim().is_empty() {
            "permissions request".to_string()
        } else {
            ctx.wire_name.trim().to_string()
        };
        let body = text_blocks(ctx.content);
        let trimmed = body.trim();
        let description = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };
        let mut fields = Vec::new();

        if let Some(cwd) = pick::<String>(ctx.raw_input, "cwd").filter(|cwd| !cwd.is_empty()) {
            fields.push(ToolField {
                label: "cwd".into(),
                value: cwd,
            });
        }

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

pub fn matches(ctx: &FormatterContext) -> bool {
    ctx.raw_input.and_then(|raw| raw.get("permissions")).is_some()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::PermissionsFormatter;
    use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};

    #[test]
    fn content_summary_becomes_description() {
        let raw = json!({
            "permissions": { "network": { "enabled": true } },
            "cwd": "/tmp/project"
        });
        let content = vec![json!({
            "type": "content",
            "content": {
                "type": "text",
                "text": "Network Access: true"
            }
        })];
        let ctx = FormatterContext {
            wire_name: "Permissions Request",
            tool_kind: &crate::tools::ToolKind::Other,
            raw_input: Some(&raw),
            adapter: "acp-codex",
            content: &content,
            started_at: 0,
            completed_at: None,
        };
        let formatted = PermissionsFormatter.format(&ctx);

        assert_eq!(formatted.title, "Permissions Request");
        assert_eq!(formatted.description.as_deref(), Some("Network Access: true"));
        assert!(formatted
            .fields
            .iter()
            .any(|field| field.label == "cwd" && field.value == "/tmp/project"));
    }
}
