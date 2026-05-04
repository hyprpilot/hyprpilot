//! `other` — final fallback when no per-adapter override matches and
//! the wire `kind` isn't in the closed ACP-spec set. Title from the
//! agent's wire name; rawInput projected to fields verbatim;
//! `description` (the captain-facing summary every claude-code /
//! opencode tool emits as a separate rawInput key) extracts to the
//! formatted `description` so the UI renders it as markdown ABOVE the
//! field grid.

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::{args_to_fields, pick, dedupe_output, title_prefix};
use crate::tools::formatter::types::FormattedToolCall;

pub struct OtherFormatter;

impl ToolFormatter for OtherFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let title = title_prefix(ctx.wire_name, "tool");

        let description = pick::<String>(ctx.raw_input, "description").filter(|s| !s.is_empty());

        let fields = args_to_fields(ctx.raw_input, &["description"]);

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
