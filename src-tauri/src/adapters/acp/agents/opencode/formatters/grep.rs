//! opencode's `grep` tool. RawInput: `{ pattern, path?, include? }`.

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::{dedupe_output, pick};
use crate::tools::formatter::types::{FormattedToolCall, ToolField};

pub struct GrepFormatter;

impl ToolFormatter for GrepFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let pattern = pick::<String>(ctx.raw_input, "pattern").filter(|s| !s.is_empty());
        let path = pick::<String>(ctx.raw_input, "path").filter(|s| !s.is_empty());
        let include = pick::<String>(ctx.raw_input, "include").filter(|s| !s.is_empty());

        let title = match (pattern.as_deref(), path.as_deref()) {
            (Some(p), Some(root)) => format!("grep · {} in {}", p, root),
            (Some(p), None) => format!("grep · {}", p),
            _ => "grep".to_string(),
        };

        let mut fields: Vec<ToolField> = Vec::new();
        if let Some(p) = pattern {
            fields.push(ToolField {
                label: "pattern".into(),
                value: p,
            });
        }
        if let Some(p) = path {
            fields.push(ToolField {
                label: "path".into(),
                value: p,
            });
        }
        if let Some(i) = include {
            fields.push(ToolField {
                label: "include".into(),
                value: i,
            });
        }

        FormattedToolCall {
            title,
            stats: Vec::new(),
            description: None,
            diff: None,
            output: dedupe_output(ctx.content, None),
            fields,
        }
    }
}
