//! claude-code's `WebSearch` tool. Title carries only the query.
//! Domain allow/block lists ride as structured fields where the
//! captain reads them as comma-joined values rather than a long
//! parenthetical jammed into the title.

use crate::tools::formatter::registry::{FormatterContext, FormatterRegistry, ToolFormatter};
use crate::tools::formatter::shared::{duration_stats, pick, text_blocks};
use crate::tools::formatter::types::{FormattedToolCall, ToolField};

pub struct WebSearchFormatter;

impl ToolFormatter for WebSearchFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let query = pick::<String>(ctx.raw_input, "query").filter(|s| !s.is_empty());
        let allowed = pick::<Vec<String>>(ctx.raw_input, "allowed_domains").unwrap_or_default();
        let blocked = pick::<Vec<String>>(ctx.raw_input, "blocked_domains").unwrap_or_default();

        let title = match query {
            Some(q) => format!("search · {}", q),
            None => "search".to_string(),
        };

        let mut fields: Vec<ToolField> = Vec::new();
        if !allowed.is_empty() {
            fields.push(ToolField {
                label: "allowed".to_string(),
                value: allowed.join(", "),
            });
        }
        if !blocked.is_empty() {
            fields.push(ToolField {
                label: "blocked".to_string(),
                value: blocked.join(", "),
            });
        }

        let body = text_blocks(ctx.content);
        let output = if body.is_empty() { None } else { Some(body) };

        let stats = duration_stats(ctx);

        FormattedToolCall {
            title,
            stats,
            description: None,
            diff: None,
            output,
            fields,
        }
    }
}

pub fn register(reg: &mut FormatterRegistry, adapter: &str) {
    reg.register_adapter(adapter, "WebSearch", Box::new(WebSearchFormatter));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx<'a>(raw: &'a serde_json::Value, content: &'a [serde_json::Value]) -> FormatterContext<'a> {
        FormatterContext {
            wire_name: "WebSearch",
            identity: &crate::adapters::ToolIdentity::Native,
            kind: "search",
            raw_input: Some(raw),
            adapter: "acp-claude-code",
            content,
            started_at: 0,
            completed_at: None,
        }
    }

    #[test]
    fn web_search_query_in_title_domains_in_fields() {
        let raw = json!({
            "query": "rust async drop",
            "allowed_domains": ["doc.rust-lang.org", "docs.rs"],
            "blocked_domains": ["spam.example"],
        });
        let content: Vec<serde_json::Value> = Vec::new();
        let out = WebSearchFormatter.format(&ctx(&raw, &content));

        assert_eq!(out.title, "search · rust async drop");
        let pairs: Vec<(&str, &str)> = out
            .fields
            .iter()
            .map(|f| (f.label.as_str(), f.value.as_str()))
            .collect();

        assert!(pairs.contains(&("allowed", "doc.rust-lang.org, docs.rs")));
        assert!(pairs.contains(&("blocked", "spam.example")));
    }

    #[test]
    fn web_search_without_domains_emits_no_fields() {
        let raw = json!({ "query": "x" });
        let content: Vec<serde_json::Value> = Vec::new();
        let out = WebSearchFormatter.format(&ctx(&raw, &content));

        assert!(out.fields.is_empty());
    }
}
