//! opencode plan-exit tool.

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::{duration_stats, text_blocks};
use crate::tools::formatter::types::FormattedToolCall;

pub struct PlanExitFormatter;

impl ToolFormatter for PlanExitFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let block_text = text_blocks(ctx.content);
        let trimmed = block_text.trim();

        FormattedToolCall {
            title: "plan exit".into(),
            stats: duration_stats(ctx),
            description: None,
            diff: None,
            output: (!trimmed.is_empty()).then(|| trimmed.to_string()),
            fields: Vec::new(),
        }
    }
}
