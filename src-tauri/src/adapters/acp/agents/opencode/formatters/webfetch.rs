//! opencode's `webfetch` tool. RawInput: `{ url, format?, timeout? }`.

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::{pick, text_blocks};
use crate::tools::formatter::types::{FormattedToolCall, ToolField};

pub struct WebFetchFormatter;

impl ToolFormatter for WebFetchFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let url = pick::<String>(ctx.raw_input, "url").filter(|s| !s.is_empty());
        let format = pick::<String>(ctx.raw_input, "format").filter(|s| !s.is_empty());

        let title = match url.as_deref() {
            Some(u) => format!("fetch · {}", short_host(u)),
            None => "fetch".to_string(),
        };

        let mut fields: Vec<ToolField> = Vec::new();
        if let Some(u) = url {
            fields.push(ToolField {
                label: "url".into(),
                value: u,
            });
        }
        if let Some(f) = format {
            fields.push(ToolField {
                label: "format".into(),
                value: f,
            });
        }

        let block_text = text_blocks(ctx.content);
        let trimmed = block_text.trim();
        let output = if trimmed.is_empty() { None } else { Some(trimmed.to_string()) };

        FormattedToolCall {
            title,
            stat: None,
            description: None,
            output,
            fields,
        }
    }
}

fn short_host(url: &str) -> String {
    let trimmed = url.trim_start_matches("https://").trim_start_matches("http://");
    trimmed.split('/').next().unwrap_or(url).to_string()
}
