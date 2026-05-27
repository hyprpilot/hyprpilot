//! opencode plan-exit tool.

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::{dedupe_output, duration_stats};
use crate::tools::formatter::types::FormattedToolCall;

pub struct PlanExitFormatter;

impl ToolFormatter for PlanExitFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        FormattedToolCall {
            title: "plan exit".into(),
            stats: duration_stats(ctx),
            description: None,
            diff: None,
            output: dedupe_output(ctx.content, None),
            fields: Vec::new(),
        }
    }
}
