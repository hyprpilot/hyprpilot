//! opencode's `patch` (apply_patch) tool. RawInput: `{ patchText }`.
//! The actual diff lives in `metadata.diff`; the patchText is the
//! agent's input. Title shows a chars-summary of the patch.

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::{pick, text_blocks};
use crate::tools::formatter::types::{FormattedToolCall, ToolField};

pub struct PatchFormatter;

impl ToolFormatter for PatchFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let patch = pick::<String>(ctx.raw_input, "patchText").filter(|s| !s.is_empty());
        let title = "patch".to_string();
        let stat = patch.as_deref().map(|p| format!("{} chars", p.len()));

        let description = patch.map(|p| format!("```diff\n{}\n```", p));

        let block_text = text_blocks(ctx.content);
        let trimmed = block_text.trim();
        let output = if trimmed.is_empty() { None } else { Some(trimmed.to_string()) };

        FormattedToolCall {
            title,
            stat,
            description,
            output,
            fields: Vec::<ToolField>::new(),
        }
    }
}
