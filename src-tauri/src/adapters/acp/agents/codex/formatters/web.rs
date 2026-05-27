//! codex-acp's web-search tool. Initial title is `Searching the Web`;
//! later updates keep the same tool-call id and carry
//! `rawInput.action` (`{ type: "search" | "open_page" |
//! "find_in_page", ... }`) with the live query / URL / pattern.

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::pick;
use crate::tools::formatter::types::{FormattedToolCall, ToolField};

pub struct WebSearchFormatter;

impl ToolFormatter for WebSearchFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let title = title(ctx).unwrap_or_else(|| {
            if ctx.wire_name.trim().is_empty() {
                "web search".to_string()
            } else {
                ctx.wire_name.trim().to_string()
            }
        });
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
            diff: None,
            output: None,
            fields,
        }
    }
}

fn title(ctx: &FormatterContext) -> Option<String> {
    let action = ctx.raw_input?.get("action")?;
    match action.get("type").and_then(|v| v.as_str())? {
        "search" => action
            .get("queries")
            .and_then(|v| v.as_array())
            .map(|queries| {
                queries
                    .iter()
                    .filter_map(|query| query.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .filter(|queries| !queries.is_empty())
            .or_else(|| {
                action
                    .get("query")
                    .and_then(|v| v.as_str())
                    .filter(|query| !query.is_empty())
                    .map(str::to_string)
            })
            .map(|query| format!("Searching for: {query}"))
            .or_else(|| Some("Web search".to_string())),
        "open_page" => action
            .get("url")
            .and_then(|v| v.as_str())
            .filter(|url| !url.is_empty())
            .map(|url| format!("Opening: {url}"))
            .or_else(|| Some("Open page".to_string())),
        "find_in_page" => {
            let pattern = action.get("pattern").and_then(|v| v.as_str()).filter(|v| !v.is_empty());
            let url = action.get("url").and_then(|v| v.as_str()).filter(|v| !v.is_empty());
            Some(match (pattern, url) {
                (Some(pattern), Some(url)) => format!("Finding: {pattern} in {url}"),
                (Some(pattern), None) => format!("Finding: {pattern}"),
                (None, Some(url)) => format!("Find in page: {url}"),
                (None, None) => "Find in page".to_string(),
            })
        }
        _ => Some("Web search".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::WebSearchFormatter;
    use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};

    #[test]
    fn raw_action_updates_frozen_initial_title() {
        let raw = json!({
            "query": "rust",
            "action": {
                "type": "find_in_page",
                "pattern": "FormatterContext",
                "url": "https://example.test"
            }
        });
        let ctx = FormatterContext {
            wire_name: "Searching the Web",
            tool_kind: &crate::tools::ToolKind::Fetch,
            raw_input: Some(&raw),
            adapter: "acp-codex",
            content: &[],
            started_at: 0,
            completed_at: None,
        };
        let formatted = WebSearchFormatter.format(&ctx);

        assert_eq!(formatted.title, "Finding: FormatterContext in https://example.test");
    }
}
