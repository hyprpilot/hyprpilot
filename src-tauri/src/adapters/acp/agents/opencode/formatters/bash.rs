//! opencode's `bash` shell tool. RawInput: `{ command, description,
//! timeout?, workdir? }`. Title surfaces the leading command token;
//! the LLM-supplied `description` rides into `description`; full
//! command into the description's fenced block; stdout/stderr into
//! `output` (streamed; opencode dedupes via hash).

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::{duration_stats, pick, text_blocks};
use crate::tools::formatter::types::{FormattedToolCall, ToolField};

pub struct BashFormatter;

impl ToolFormatter for BashFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let command = pick::<String>(ctx.raw_input, "command").filter(|s| !s.is_empty());
        let summary = pick::<String>(ctx.raw_input, "description").filter(|s| !s.is_empty());
        let timeout = pick::<i64>(ctx.raw_input, "timeout");
        let workdir = pick::<String>(ctx.raw_input, "workdir").filter(|s| !s.is_empty());

        let title = match command.as_deref() {
            Some(cmd) => {
                let head = cmd.split_whitespace().next().unwrap_or("bash");
                format!("bash · {}", head)
            }
            None => "bash".to_string(),
        };

        let mut parts: Vec<String> = Vec::new();
        if let Some(s) = summary.as_deref() {
            parts.push(s.to_string());
        }
        if let Some(cmd) = command.as_deref() {
            parts.push(format!("```bash\n{}\n```", cmd));
        }
        let description = if parts.is_empty() {
            None
        } else {
            Some(parts.join("\n\n"))
        };

        let mut fields: Vec<ToolField> = Vec::new();
        if let Some(t) = timeout {
            fields.push(ToolField {
                label: "timeout".into(),
                value: t.to_string(),
            });
        }
        if let Some(w) = workdir {
            fields.push(ToolField {
                label: "workdir".into(),
                value: w,
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
            description,
            output,
            fields,
        }
    }
}
