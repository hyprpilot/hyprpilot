//! codex-acp's `Guardian Review` — assessment-style think-call.
//! Title is the literal `Guardian Review`. Content carries
//! `"Status: <state>"` lines built by codex's
//! `guardian_assessment_content` helper. We surface the body as
//! `description` so it renders as markdown.

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::{args_to_fields, text_blocks};
use crate::tools::formatter::types::FormattedToolCall;

pub struct GuardianFormatter;

impl ToolFormatter for GuardianFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let body = text_blocks(ctx.content);
        let trimmed = body.trim();
        let description = if trimmed.is_empty() { None } else { Some(trimmed.to_string()) };
        let fields = args_to_fields(ctx.raw_input, &[]);

        FormattedToolCall {
            title: "guardian review".to_string(),
            stat: None,
            description,
            output: None,
            fields,
        }
    }
}
