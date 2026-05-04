//! claude-code's `Bash` + `BashOutput` tools (kind: execute).
//! Title surfaces the leading command token; full command rides into
//! `description` as a fenced bash block.

use crate::tools::formatter::registry::{FormatterContext, FormatterRegistry, ToolFormatter};
use crate::tools::formatter::shared::{dedupe_output, pick};
use crate::tools::formatter::types::{FormattedToolCall, ToolField};

pub struct BashFormatter;

impl ToolFormatter for BashFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let raw = ctx.raw_input;
        let command = pick::<String>(raw, "command").filter(|s| !s.is_empty());
        let description = pick::<String>(raw, "description").filter(|s| !s.is_empty());
        let is_background = pick::<bool>(raw, "is_background").unwrap_or(false);
        let id = pick::<String>(raw, "bash_id").or_else(|| pick(raw, "shell_id"));
        let filter = pick::<String>(raw, "filter").filter(|s| !s.is_empty());

        let title = if let Some(cmd) = command.as_deref() {
            let head = cmd.split_whitespace().next().unwrap_or("bash");
            if is_background {
                format!("bash · {} (background)", head)
            } else {
                format!("bash · {}", head)
            }
        } else if let Some(id) = id.as_deref() {
            match filter.as_deref() {
                Some(f) => format!("bash · tail #{} — filter {}", id, f),
                None => format!("bash · tail #{}", id),
            }
        } else {
            "bash".to_string()
        };

        let mut fields: Vec<ToolField> = Vec::new();
        if let Some(id) = id {
            fields.push(ToolField {
                label: "shell".to_string(),
                value: id,
            });
        }
        if let Some(f) = filter {
            fields.push(ToolField {
                label: "filter".to_string(),
                value: f,
            });
        }

        let mut parts: Vec<String> = Vec::new();
        if let Some(d) = description.as_deref() {
            parts.push(d.to_string());
        }
        if let Some(cmd) = command.as_deref() {
            parts.push(format!("```bash\n{}\n```", cmd));
        }
        let body = if parts.is_empty() {
            None
        } else {
            Some(parts.join("\n\n"))
        };

        let output = dedupe_output(ctx.content, description.as_deref());

        FormattedToolCall {
            title,
            stat: None,
            description: body,
            output,
            fields,
        }
    }
}

pub fn register(reg: &mut FormatterRegistry, adapter: &str) {
    reg.register_adapter(adapter, "Bash", Box::new(BashFormatter));
    reg.register_adapter(adapter, "BashOutput", Box::new(BashFormatter));
}
