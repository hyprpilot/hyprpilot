//! opencode's `todowrite` tool. RawInput: `{ todos: [{ content,
//! status, priority }] }`. Mirrors the claude-code todo formatter.

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::pick;
use crate::tools::formatter::types::{FormattedToolCall, ToolField};

#[derive(serde::Deserialize)]
struct TodoEntry {
    content: Option<String>,
    status: Option<String>,
    priority: Option<String>,
}

pub struct TodoFormatter;

impl ToolFormatter for TodoFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let todos: Vec<TodoEntry> = pick(ctx.raw_input, "todos").unwrap_or_default();

        let pending = todos
            .iter()
            .filter(|t| matches!(t.status.as_deref(), Some("pending") | Some("in_progress")))
            .count();

        let title = format!("todo · {} pending", pending);
        let stat = Some(format!("{}/{}", pending, todos.len()));

        let mut description = String::new();
        for t in &todos {
            let mark = match t.status.as_deref() {
                Some("completed") => "[x]",
                Some("in_progress") => "[~]",
                Some("cancelled") => "[-]",
                _ => "[ ]",
            };
            let body = t.content.as_deref().unwrap_or("");
            description.push_str(&format!("- {} {}\n", mark, body));
        }

        let mut fields: Vec<ToolField> = Vec::new();
        for t in &todos {
            if let Some(p) = t.priority.as_deref().filter(|s| !s.is_empty()) {
                let body = t.content.as_deref().unwrap_or("");
                fields.push(ToolField {
                    label: p.to_string(),
                    value: body.to_string(),
                });
            }
        }

        FormattedToolCall {
            title,
            stat,
            description: if description.is_empty() {
                None
            } else {
                Some(description)
            },
            output: None,
            fields,
        }
    }
}
