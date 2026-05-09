//! claude-code's `Grep` tool. Title carries the search pattern; path
//! / glob / type / output_mode / case-flags ride into `fields` where
//! the captain can read them as a structured arg list. Without this
//! split, the title balloons to `grep · pattern · in /path/sub/dir
//! glob=*.rs type=rust mode=content -i -n` and gets ellipsised in
//! the pill header.

use serde_json::Value;

use crate::tools::formatter::registry::{FormatterContext, FormatterRegistry, ToolFormatter};
use crate::tools::formatter::shared::{pick, short_path, text_blocks};
use crate::tools::formatter::types::{FormattedToolCall, ToolField};

pub struct GrepFormatter;

impl ToolFormatter for GrepFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let raw = ctx.raw_input;
        let pattern = pick::<String>(raw, "pattern").filter(|s| !s.is_empty());
        let path = pick::<String>(raw, "path").filter(|s| !s.is_empty());
        let glob = pick::<String>(raw, "glob").filter(|s| !s.is_empty());
        let typ = pick::<String>(raw, "type").filter(|s| !s.is_empty());
        let output_mode = pick::<String>(raw, "output_mode").filter(|s| !s.is_empty());

        let title = match pattern.as_deref() {
            Some(p) => format!("grep · {}", p),
            None => "grep".to_string(),
        };

        let mut fields: Vec<ToolField> = Vec::new();
        if let Some(p) = path {
            fields.push(ToolField {
                label: "path".to_string(),
                value: short_path(&p),
            });
        }
        if let Some(g) = glob {
            fields.push(ToolField {
                label: "glob".to_string(),
                value: g,
            });
        }
        if let Some(t) = typ {
            fields.push(ToolField {
                label: "type".to_string(),
                value: t,
            });
        }
        if let Some(m) = output_mode {
            fields.push(ToolField {
                label: "mode".to_string(),
                value: m,
            });
        }
        if let Some(obj) = raw.and_then(Value::as_object) {
            let mut flags: Vec<&str> = Vec::new();
            if matches!(obj.get("-i"), Some(Value::Bool(true))) {
                flags.push("-i");
            }
            if matches!(obj.get("-n"), Some(Value::Bool(true))) {
                flags.push("-n");
            }
            if !flags.is_empty() {
                fields.push(ToolField {
                    label: "flags".to_string(),
                    value: flags.join(" "),
                });
            }
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
    reg.register_adapter(adapter, "Grep", Box::new(GrepFormatter));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx<'a>(raw: &'a serde_json::Value, content: &'a [serde_json::Value]) -> FormatterContext<'a> {
        FormatterContext {
            wire_name: "Grep",
            kind: "search",
            raw_input: Some(raw),
            adapter: "acp-claude-code",
            content,
            started_at: 0,
            completed_at: None,
        }
    }

    /// Pin the post-fix shape: title carries only the pattern; path /
    /// glob / type / mode / flags ride as structured fields. The old
    /// "everything jammed into title" balloon got ellipsised and made
    /// it impossible for the captain to read what was actually being
    /// searched.
    #[test]
    fn grep_args_split_into_title_and_fields() {
        let raw = json!({
            "pattern": "fn main",
            "path": "/home/u/proj/src",
            "glob": "*.rs",
            "type": "rust",
            "output_mode": "content",
            "-i": true,
            "-n": true,
        });
        let content: Vec<serde_json::Value> = Vec::new();
        let out = GrepFormatter.format(&ctx(&raw, &content));

        assert_eq!(out.title, "grep · fn main");
        let pairs: Vec<(&str, &str)> = out
            .fields
            .iter()
            .map(|f| (f.label.as_str(), f.value.as_str()))
            .collect();
        // path is short_path-shortened; assert by key presence + value substring.
        let path_value = pairs.iter().find(|(l, _)| *l == "path").expect("path field").1;
        assert!(path_value.ends_with("src"), "path field shortened: {path_value}");
        assert!(pairs.contains(&("glob", "*.rs")));
        assert!(pairs.contains(&("type", "rust")));
        assert!(pairs.contains(&("mode", "content")));
        assert!(pairs.contains(&("flags", "-i -n")));
        assert!(out.description.is_none(), "grep has no description");
    }

    #[test]
    fn grep_without_pattern_falls_back_to_bare_label() {
        let raw = json!({ "path": "/tmp" });
        let content: Vec<serde_json::Value> = Vec::new();
        let out = GrepFormatter.format(&ctx(&raw, &content));

        assert_eq!(out.title, "grep");
    }

    #[test]
    fn grep_omits_field_when_arg_absent() {
        let raw = json!({ "pattern": "x" });
        let content: Vec<serde_json::Value> = Vec::new();
        let out = GrepFormatter.format(&ctx(&raw, &content));

        // pattern only — no other args, no fields.
        assert!(
            out.fields.is_empty(),
            "no extra args → empty fields, got {:?}",
            out.fields
        );
    }
}
