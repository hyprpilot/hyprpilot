//! opencode's `lsp` tool. RawInput: `{ operation, filePath, line,
//! character, query? }`. Operation drives the title; useful op
//! values: goToDefinition, findReferences, hover, documentSymbol,
//! workspaceSymbol, goToImplementation, prepareCallHierarchy,
//! incomingCalls, outgoingCalls.

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::{dedupe_output, pick};
use crate::tools::formatter::types::{FormattedToolCall, ToolField};

pub struct LspFormatter;

impl ToolFormatter for LspFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let op = pick::<String>(ctx.raw_input, "operation").filter(|s| !s.is_empty());
        let file = pick::<String>(ctx.raw_input, "filePath").filter(|s| !s.is_empty());
        let line = pick::<i64>(ctx.raw_input, "line");
        let character = pick::<i64>(ctx.raw_input, "character");
        let query = pick::<String>(ctx.raw_input, "query").filter(|s| !s.is_empty());

        let detail = match (file.as_deref(), line, character, query.as_deref(), op.as_deref()) {
            (_, _, _, Some(q), Some("workspaceSymbol")) => Some(q.to_string()),
            (Some(f), Some(l), Some(c), _, _) => Some(format!("{}:{}:{}", f, l, c)),
            (Some(f), _, _, _, _) => Some(f.to_string()),
            _ => None,
        };

        let title = match (op.as_deref(), detail.as_deref()) {
            (Some(o), Some(d)) => format!("lsp · {} {}", o, d),
            (Some(o), None) => format!("lsp · {}", o),
            _ => "lsp".to_string(),
        };

        let mut fields: Vec<ToolField> = Vec::new();
        if let Some(o) = op {
            fields.push(ToolField {
                label: "operation".into(),
                value: o,
            });
        }
        if let Some(f) = file {
            fields.push(ToolField {
                label: "file".into(),
                value: f,
            });
        }
        if let Some(q) = query {
            fields.push(ToolField {
                label: "query".into(),
                value: q,
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
