use std::collections::BTreeMap;

use serde_json::Value;

use crate::tools::formatter::registry::{FormatterContext, FormatterRegistry, ToolFormatter};
use crate::tools::formatter::shared::{pick, text_blocks};
use crate::tools::formatter::types::{FormattedToolCall, Stat, ToolField};

pub struct TodoFormatter;

impl ToolFormatter for TodoFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let todos: Vec<Value> = pick(ctx.raw_input, "todos").unwrap_or_default();

        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        let mut fields: Vec<ToolField> = Vec::new();
        for entry in &todos {
            let obj = match entry.as_object() {
                Some(o) => o,
                None => continue,
            };
            let status = obj.get("status").and_then(|v| v.as_str()).unwrap_or("");
            if !status.is_empty() {
                *counts.entry(status.to_string()).or_insert(0) += 1;
            }
            let text = obj
                .get("content")
                .and_then(|v| v.as_str())
                .or_else(|| obj.get("activeForm").and_then(|v| v.as_str()));
            if let Some(t) = text {
                let label = if status.is_empty() {
                    "todo".to_string()
                } else {
                    status.to_string()
                };
                fields.push(ToolField {
                    label,
                    value: t.to_string(),
                });
            }
        }

        let count = todos.len();
        let title = if count > 0 {
            format!("todos · {} {}", count, if count == 1 { "item" } else { "items" })
        } else {
            "todos".to_string()
        };
        let breakdown: Vec<String> = counts.iter().map(|(k, v)| format!("{}:{}", k, v)).collect();
        let stats: Vec<Stat> = if breakdown.is_empty() {
            Vec::new()
        } else {
            vec![Stat::Text {
                value: breakdown.join(" "),
            }]
        };

        let body = text_blocks(ctx.content);
        let output = if body.is_empty() { None } else { Some(body) };

        FormattedToolCall {
            title,
            stats,
            description: None,
            output,
            fields,
        }
    }
}

pub fn register(reg: &mut FormatterRegistry, adapter: &str) {
    reg.register_adapter(adapter, "TodoWrite", Box::new(TodoFormatter));
}
