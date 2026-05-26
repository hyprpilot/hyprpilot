//! claude-code's `Glob` tool. Title carries the pattern; search root
//! rides as a `path` field so the captain reads it as a structured
//! arg pair instead of a long string crammed into the title.

use crate::tools::formatter::registry::{FormatterContext, FormatterRegistry, ToolFormatter};
use crate::tools::formatter::shared::{pick, text_blocks};
use crate::tools::formatter::types::{FormattedToolCall, ToolField};

pub struct GlobFormatter;

impl ToolFormatter for GlobFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let pattern = pick::<String>(ctx.raw_input, "pattern").filter(|s| !s.is_empty());
        let path = pick::<String>(ctx.raw_input, "path").filter(|s| !s.is_empty());

        let title = match pattern.as_deref() {
            Some(p) => format!("glob · {}", p),
            None => "glob".to_string(),
        };

        let mut fields: Vec<ToolField> = Vec::new();
        if let Some(p) = path {
            fields.push(ToolField {
                label: "path".to_string(),
                value: p,
            });
        }

        let body = text_blocks(ctx.content);
        let output = if body.is_empty() { None } else { Some(body) };

        FormattedToolCall {
            title,
            stats: Vec::new(),
            description: None,
            diff: None,
            output,
            fields,
        }
    }
}

pub fn register(reg: &mut FormatterRegistry, adapter: &str) {
    reg.register_adapter(adapter, "Glob", Box::new(GlobFormatter));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx<'a>(raw: &'a serde_json::Value, content: &'a [serde_json::Value]) -> FormatterContext<'a> {
        FormatterContext {
            wire_name: "Glob",
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
    fn glob_pattern_in_title_path_in_field() {
        let raw = json!({ "pattern": "**/*.rs", "path": "/home/u/proj/src" });
        let content: Vec<serde_json::Value> = Vec::new();
        let out = GlobFormatter.format(&ctx(&raw, &content));

        assert_eq!(out.title, "glob · **/*.rs");
        let path_field = out.fields.iter().find(|f| f.label == "path").expect("path field");

        assert_eq!(path_field.value, "/home/u/proj/src");
    }

    #[test]
    fn glob_without_path_omits_field() {
        let raw = json!({ "pattern": "*.toml" });
        let content: Vec<serde_json::Value> = Vec::new();
        let out = GlobFormatter.format(&ctx(&raw, &content));

        assert_eq!(out.title, "glob · *.toml");
        assert!(out.fields.is_empty());
    }
}
