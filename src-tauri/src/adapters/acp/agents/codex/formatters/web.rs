//! codex-acp's web-search tool. Title evolves through several shapes
//! during a single call: `Searching the Web` → `Searching for: q` →
//! `Opening: url` → `Finding: pattern in url`. RawInput carries
//! `{ query, action }`. Pass title verbatim and surface query/action
//! as fields.

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::pick;
use crate::tools::formatter::types::{FormattedToolCall, ToolField};

pub struct WebSearchFormatter;

impl ToolFormatter for WebSearchFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let title = if ctx.wire_name.trim().is_empty() {
            "web search".to_string()
        } else {
            ctx.wire_name.trim().to_string()
        };

        let query = pick::<String>(ctx.raw_input, "query").filter(|s| !s.is_empty());

        let mut fields: Vec<ToolField> = Vec::new();
        if let Some(q) = query {
            fields.push(ToolField {
                label: "query".into(),
                value: q,
            });
        }
        if let Some(action) = ctx.raw_input.and_then(|v| v.get("action")) {
            if !action.is_null() {
                fields.push(ToolField {
                    label: "action".into(),
                    value: serde_json::to_string(action).unwrap_or_default(),
                });
            }
        }

        FormattedToolCall {
            title,
            stats: Vec::new(),
            description: None,
            output: None,
            fields,
        }
    }
}
