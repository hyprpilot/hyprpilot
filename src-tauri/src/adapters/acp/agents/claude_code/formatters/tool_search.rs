//! claude-code's `ToolSearch` tool. Title carries the query; non-
//! default `max_results` rides as a structured field rather than a
//! parenthetical title suffix.

use crate::tools::formatter::registry::{FormatterContext, FormatterRegistry, ToolFormatter};
use crate::tools::formatter::shared::{pick, text_blocks};
use crate::tools::formatter::types::{FormattedToolCall, ToolField};

pub struct ToolSearchFormatter;

impl ToolFormatter for ToolSearchFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let query = pick::<String>(ctx.raw_input, "query").filter(|s| !s.is_empty());
        let max = pick::<i64>(ctx.raw_input, "max_results");

        let title = match query {
            Some(q) => format!("tool search · {}", q),
            None => "tool search".to_string(),
        };

        let mut fields: Vec<ToolField> = Vec::new();
        if let Some(m) = max.filter(|m| *m > 0 && *m != 5) {
            fields.push(ToolField {
                label: "max results".to_string(),
                value: m.to_string(),
            });
        }

        let body = text_blocks(ctx.content);
        let output = if body.is_empty() { None } else { Some(body) };

        FormattedToolCall {
            title,
            stats: Vec::new(),
            description: None,
            output,
            fields,
        }
    }
}

pub fn register(reg: &mut FormatterRegistry, adapter: &str) {
    reg.register_adapter(adapter, "ToolSearch", Box::new(ToolSearchFormatter));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx<'a>(raw: &'a serde_json::Value, content: &'a [serde_json::Value]) -> FormatterContext<'a> {
        FormatterContext {
            wire_name: "ToolSearch",
            kind: "search",
            raw_input: Some(raw),
            adapter: "acp-claude-code",
            content,
            started_at: 0,
            completed_at: None,
        }
    }

    #[test]
    fn tool_search_query_in_title_max_in_field_when_non_default() {
        let raw = json!({ "query": "playwright", "max_results": 20 });
        let content: Vec<serde_json::Value> = Vec::new();
        let out = ToolSearchFormatter.format(&ctx(&raw, &content));

        assert_eq!(out.title, "tool search · playwright");
        let max_field = out.fields.iter().find(|f| f.label == "max results");

        assert_eq!(max_field.map(|f| f.value.as_str()), Some("20"));
    }

    #[test]
    fn tool_search_omits_max_field_at_default() {
        let raw = json!({ "query": "x", "max_results": 5 });
        let content: Vec<serde_json::Value> = Vec::new();
        let out = ToolSearchFormatter.format(&ctx(&raw, &content));

        assert!(out.fields.is_empty(), "default max_results=5 → no field");
    }
}
