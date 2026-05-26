//! claude-code's `Terminal` tool. Title surfaces the leading command
//! token only (otherwise the long command — `python3 -c "..."`,
//! pipelines, multi-line scripts — gets crammed into the pill header
//! and ellipsised). Full command rides into `description` as a fenced
//! `bash` block where the captain can read it. Terminal id rides as
//! a field. Mirrors the structure of the `Bash` formatter.

use crate::tools::formatter::registry::{FormatterContext, FormatterRegistry, ToolFormatter};
use crate::tools::formatter::shared::{duration_stats, pick, text_blocks};
use crate::tools::formatter::types::{FormattedToolCall, ToolField};

pub struct TerminalFormatter;

impl ToolFormatter for TerminalFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let tid = pick::<String>(ctx.raw_input, "terminal_id").filter(|s| !s.is_empty());
        let command = pick::<String>(ctx.raw_input, "command").filter(|s| !s.is_empty());

        // Title carries the FIRST whitespace-delimited token of the
        // command (the program name) — short enough to live inside
        // the pill header without ellipsis. Full command lives in
        // `description`.
        let head = command
            .as_deref()
            .and_then(|c| c.split_whitespace().next())
            .unwrap_or("terminal");
        let title = match (command.as_deref(), tid.as_deref()) {
            (Some(_), Some(id)) => format!("terminal #{} · {}", id, head),
            (Some(_), None) => format!("terminal · {}", head),
            (None, Some(id)) => format!("terminal #{}", id),
            (None, None) => "terminal".to_string(),
        };

        let description = command.as_deref().map(|cmd| format!("```bash\n{}\n```", cmd));

        let mut fields: Vec<ToolField> = Vec::new();
        if let Some(id) = tid {
            fields.push(ToolField {
                label: "terminal".to_string(),
                value: id,
            });
        }

        let body = text_blocks(ctx.content);
        let output = if body.is_empty() { None } else { Some(body) };

        let stats = duration_stats(ctx);

        FormattedToolCall {
            title,
            stats,
            description,
            diff: None,
            output,
            fields,
        }
    }
}

pub fn register(reg: &mut FormatterRegistry, adapter: &str) {
    reg.register_adapter(adapter, "Terminal", Box::new(TerminalFormatter));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx<'a>(raw: &'a serde_json::Value, content: &'a [serde_json::Value]) -> FormatterContext<'a> {
        FormatterContext {
            wire_name: "Terminal",
            identity: &crate::adapters::ToolIdentity::Native,
            kind: "execute",
            raw_input: Some(raw),
            adapter: "acp-claude-code",
            content,
            started_at: 0,
            completed_at: None,
        }
    }

    /// Pin the post-fix shape: title is the SHORT command head (program
    /// name), full command lives in `description` as a fenced bash
    /// block, terminal id rides as a field. Without this, the long
    /// command crammed into the title gets ellipsised in the pill
    /// header and the captain can't see what's running.
    #[test]
    fn terminal_command_lands_in_description_not_title() {
        let raw = json!({
            "command": "echo \"PI computed\" && python3 -c 'long script here'",
            "terminal_id": "abc123",
        });
        let content: Vec<serde_json::Value> = Vec::new();
        let out = TerminalFormatter.format(&ctx(&raw, &content));

        // Title is short — head only, plus the terminal id badge.
        assert_eq!(out.title, "terminal #abc123 · echo");
        // Full command in description as a fenced block.
        let desc = out.description.as_deref().expect("description must be populated");
        assert!(
            desc.contains("```bash\n") && desc.contains("python3 -c"),
            "description must hold the full command in a fenced block, got: {desc:?}"
        );
        // Terminal id surfaces as a field.
        let labels: Vec<&str> = out.fields.iter().map(|f| f.label.as_str()).collect();
        assert!(labels.contains(&"terminal"));
    }

    #[test]
    fn terminal_without_id_omits_badge() {
        let raw = json!({ "command": "ls -la" });
        let content: Vec<serde_json::Value> = Vec::new();
        let out = TerminalFormatter.format(&ctx(&raw, &content));

        assert_eq!(out.title, "terminal · ls");
        assert!(out.description.as_deref().unwrap().contains("ls -la"));
        assert!(out.fields.is_empty(), "no terminal_id → no field");
    }

    #[test]
    fn terminal_without_command_falls_back_to_id_or_bare_label() {
        let content: Vec<serde_json::Value> = Vec::new();
        let raw_with_id = json!({ "terminal_id": "xyz" });
        let raw_empty = json!({});

        assert_eq!(
            TerminalFormatter.format(&ctx(&raw_with_id, &content)).title,
            "terminal #xyz"
        );
        assert_eq!(TerminalFormatter.format(&ctx(&raw_empty, &content)).title, "terminal");
    }

    #[test]
    fn terminal_output_carries_text_blocks() {
        let raw = json!({ "command": "echo hi" });
        let content = vec![json!({ "type": "content", "content": { "type": "text", "text": "hi\n" } })];
        let out = TerminalFormatter.format(&ctx(&raw, &content));

        assert_eq!(out.output.as_deref(), Some("hi\n"));
    }
}
