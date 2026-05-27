//! opencode permission-only pseudo tools.

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::{args_to_fields, pick};
use crate::tools::formatter::types::FormattedToolCall;

pub struct ExternalDirectoryFormatter;

impl ToolFormatter for ExternalDirectoryFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let path = pick::<String>(ctx.raw_input, "path").filter(|s| !s.is_empty());
        let title = path
            .as_deref()
            .map(|path| format!("external directory · {path}"))
            .unwrap_or_else(|| "external directory".to_string());

        FormattedToolCall {
            title,
            stats: Vec::new(),
            description: None,
            diff: None,
            output: None,
            fields: args_to_fields(ctx.raw_input, &[]),
        }
    }
}

pub struct WorkflowApprovalFormatter;

impl ToolFormatter for WorkflowApprovalFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let tools = pick::<Vec<serde_json::Value>>(ctx.raw_input, "tools").unwrap_or_default();
        let description = (!tools.is_empty()).then(|| {
            tools
                .iter()
                .filter_map(|tool| {
                    let name = tool.get("name").and_then(|v| v.as_str())?;
                    let args = tool.get("args").and_then(|v| v.as_str()).unwrap_or("");
                    if args.is_empty() {
                        Some(format!("- `{name}`"))
                    } else {
                        Some(format!("- `{name}` `{args}`"))
                    }
                })
                .collect::<Vec<_>>()
                .join("\n")
        });

        FormattedToolCall {
            title: "workflow approval".into(),
            stats: Vec::new(),
            description,
            diff: None,
            output: None,
            fields: args_to_fields(ctx.raw_input, &["tools"]),
        }
    }
}
