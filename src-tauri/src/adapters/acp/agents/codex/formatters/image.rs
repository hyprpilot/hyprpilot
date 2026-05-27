//! codex-acp's image-generation tool. The begin event is
//! `Image generation` with `rawInput: { call_id }`; the end event
//! appends content blocks (`Revised prompt: ...` text plus an image
//! block) and carries completion details in rawOutput, which the
//! formatter layer does not consume today.

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::dedupe_output;
use crate::tools::formatter::types::FormattedToolCall;

pub struct ImageFormatter;

impl ToolFormatter for ImageFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        FormattedToolCall {
            title: "image generation".to_string(),
            stats: Vec::new(),
            description: dedupe_output(ctx.content, None),
            diff: None,
            output: None,
            fields: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::ImageFormatter;
    use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};

    #[test]
    fn revised_prompt_becomes_description() {
        let content = vec![json!({
            "type": "content",
            "content": {
                "type": "text",
                "text": "Revised prompt: A tiny blue square"
            }
        })];
        let ctx = FormatterContext {
            wire_name: "Image generation",
            tool_kind: &crate::tools::ToolKind::Other,
            raw_input: None,
            adapter: "acp-codex",
            content: &content,
            started_at: 0,
            completed_at: None,
        };
        let formatted = ImageFormatter.format(&ctx);

        assert_eq!(formatted.title, "image generation");
        assert_eq!(
            formatted.description.as_deref(),
            Some("Revised prompt: A tiny blue square")
        );
    }
}
