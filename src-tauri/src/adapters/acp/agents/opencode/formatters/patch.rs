//! opencode's `patch` (apply_patch) tool. RawInput: `{ patchText }`.
//! The actual diff lives in `metadata.diff`; the patchText is the
//! agent's input. Title shows a chars-summary of the patch.

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::{pick, text_blocks};
use crate::tools::formatter::types::{FormattedToolCall, Stat, ToolField};

pub struct PatchFormatter;

impl ToolFormatter for PatchFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let patch = pick::<String>(ctx.raw_input, "patchText").filter(|s| !s.is_empty());
        let title = "patch".to_string();
        // Approximate diff stats from the patch text — count lines
        // beginning with `+ ` / `- ` (excluding the `+++` / `---` file
        // markers) for the captain's at-a-glance magnitude. Patch
        // content already lives in the description as a `diff` fence.
        let stats: Vec<Stat> = match patch.as_deref() {
            Some(p) => {
                let mut added = 0u32;
                let mut removed = 0u32;
                for line in p.lines() {
                    if line.starts_with("+++") || line.starts_with("---") {
                        continue;
                    }

                    if line.starts_with('+') {
                        added = added.saturating_add(1);
                    } else if line.starts_with('-') {
                        removed = removed.saturating_add(1);
                    }
                }

                if added == 0 && removed == 0 {
                    Vec::new()
                } else {
                    vec![Stat::Diff { added, removed }]
                }
            }
            None => Vec::new(),
        };

        let description = patch.map(|p| format!("```diff\n{}\n```", p));

        let block_text = text_blocks(ctx.content);
        let trimmed = block_text.trim();
        let output = if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        };

        FormattedToolCall {
            title,
            stats,
            description,
            output,
            fields: Vec::<ToolField>::new(),
        }
    }
}
