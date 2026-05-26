//! codex-acp's parsed-shell tool calls: `Read foo.rs`, `List /path`,
//! `Search query in path`. All carry the same `ExecCommandBeginEvent`
//! rawInput shape (`call_id`, `command`, `cwd`, `parsed_cmd: [...]`).
//! The title is already human-friendly — the formatter passes it
//! through and surfaces the streamed stdout as `output`. Fields:
//! `command` (the raw shell line), `cwd`, `process_id`.

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::{duration_stats, pick, text_blocks};
use crate::tools::formatter::types::{FormattedToolCall, ToolField};

pub struct ExecFormatter;

pub fn matches(ctx: &FormatterContext) -> bool {
    let Some(raw) = ctx.raw_input else {
        return false;
    };

    raw.get("command_string").is_some() || raw.get("command").is_some() || raw.get("parsed_cmd").is_some()
}

impl ToolFormatter for ExecFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let raw = ctx.raw_input;
        let command = command_string(raw);
        let cwd = pick::<String>(raw, "cwd").filter(|s| !s.is_empty());
        let pid = pick::<i64>(raw, "process_id");

        let title = if ctx.wire_name.trim().is_empty() {
            "exec".to_string()
        } else {
            ctx.wire_name.trim().to_string()
        };

        let mut fields: Vec<ToolField> = Vec::new();
        if let Some(cmd) = command.as_deref() {
            fields.push(ToolField {
                label: "command".into(),
                value: cmd.to_string(),
            });
        }
        if let Some(cwd) = cwd {
            fields.push(ToolField {
                label: "cwd".into(),
                value: cwd,
            });
        }
        if let Some(pid) = pid {
            fields.push(ToolField {
                label: "pid".into(),
                value: pid.to_string(),
            });
        }

        let block_text = text_blocks(ctx.content);
        let trimmed = block_text.trim();
        let output = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };

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

fn command_string(raw: Option<&serde_json::Value>) -> Option<String> {
    pick::<String>(raw, "command_string")
        .filter(|s| !s.is_empty())
        .or_else(|| pick::<String>(raw, "command").filter(|s| !s.is_empty()))
        .or_else(|| {
            let args = raw?.get("command")?.as_array()?;
            let parts: Vec<&str> = args.iter().filter_map(serde_json::Value::as_str).collect();

            (!parts.is_empty()).then(|| parts.join(" "))
        })
}
