//! `fetch` — HTTP / URI fetch. RawInput conventions: `url`
//! (claude-code WebFetch), `uri`. Optional `prompt` arg for
//! summarise-while-fetching tools.

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::{args_to_fields, pick, dedupe_output, title_prefix};
use crate::tools::formatter::types::{FormattedToolCall, ToolField};

pub struct FetchFormatter;

impl ToolFormatter for FetchFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let url = pick::<String>(ctx.raw_input, "url")
            .or_else(|| pick(ctx.raw_input, "uri"))
            .filter(|s| !s.is_empty());

        let prefix = title_prefix(ctx.wire_name, "fetch");
        let title = match url.as_deref() {
            Some(u) => format!("{} · {}", prefix, short_host(u)),
            None => prefix,
        };

        let description = pick::<String>(ctx.raw_input, "description").filter(|s| !s.is_empty());

        let mut fields = args_to_fields(ctx.raw_input, &["url", "uri", "description"]);
        if let Some(u) = url {
            fields.insert(
                0,
                ToolField {
                    label: "url".into(),
                    value: u,
                },
            );
        }

        let output = dedupe_output(ctx.content, description.as_deref());

        FormattedToolCall {
            title,
            stat: None,
            description,
            output,
            fields,
        }
    }
}

/// Strip protocol + path; keep host. `https://example.com/foo` → `example.com`.
fn short_host(url: &str) -> String {
    let trimmed = url.trim_start_matches("https://").trim_start_matches("http://");
    trimmed.split('/').next().unwrap_or(url).to_string()
}
