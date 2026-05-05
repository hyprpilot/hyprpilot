//! `fetch` — HTTP / URI fetch. RawInput conventions: `url`
//! (claude-code WebFetch), `uri`. Optional `prompt` arg for
//! summarise-while-fetching tools.

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::{args_to_fields, dedupe_output, pick, wire_title_or_fallback};
use crate::tools::formatter::types::{FormattedToolCall, ToolField};

pub struct FetchFormatter;

impl ToolFormatter for FetchFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let url = pick::<String>(ctx.raw_input, "url")
            .or_else(|| pick(ctx.raw_input, "uri"))
            .filter(|s| !s.is_empty());

        let title = wire_title_or_fallback(ctx.wire_name, "fetch");

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
