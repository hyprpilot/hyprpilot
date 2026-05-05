//! `execute` — shell / process invocation. RawInput conventions:
//! `command` (most agents), `cmd` / `script` (some MCP tools). Title
//! preserves the tool's wire name as prefix (claude-code's `bash`
//! stays `bash`, not the kind verb `execute`). Description is the
//! agent-supplied `description` text (markdown) followed by the
//! command in a fenced block.

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::{args_to_fields, dedupe_output, pick, wire_title_or_fallback};
use crate::tools::formatter::types::FormattedToolCall;

pub struct ExecuteFormatter;

impl ToolFormatter for ExecuteFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let command = pick::<String>(ctx.raw_input, "command")
            .or_else(|| pick(ctx.raw_input, "cmd"))
            .or_else(|| pick(ctx.raw_input, "script"))
            .filter(|s| !s.is_empty());
        let summary = pick::<String>(ctx.raw_input, "description").filter(|s| !s.is_empty());

        let title = wire_title_or_fallback(ctx.wire_name, "execute");

        let mut parts: Vec<String> = Vec::new();
        if let Some(s) = summary.as_deref() {
            parts.push(s.to_string());
        }
        if let Some(cmd) = command {
            parts.push(format!("```bash\n{}\n```", cmd));
        }
        let description = if parts.is_empty() {
            None
        } else {
            Some(parts.join("\n\n"))
        };

        let fields = args_to_fields(ctx.raw_input, &["command", "cmd", "script", "description"]);

        let output = dedupe_output(ctx.content, summary.as_deref());

        FormattedToolCall {
            title,
            stats: Vec::new(),
            description,
            output,
            fields,
        }
    }
}
