//! claude-code's `NotebookEdit` tool. Title carries notebook path
//! only; cell id, cell type, and edit mode ride as fields. New cell
//! source lands in `description` as a fenced python block so the
//! captain reads the impending change inline.

use crate::tools::formatter::registry::{FormatterContext, FormatterRegistry, ToolFormatter};
use crate::tools::formatter::shared::{pick, text_blocks};
use crate::tools::formatter::types::{FormattedToolCall, ToolField};

pub struct NotebookEditFormatter;

impl ToolFormatter for NotebookEditFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let path = pick::<String>(ctx.raw_input, "notebook_path").filter(|s| !s.is_empty());
        let cell_id = pick::<String>(ctx.raw_input, "cell_id").filter(|s| !s.is_empty());
        let cell_type = pick::<String>(ctx.raw_input, "cell_type").filter(|s| !s.is_empty());
        let edit_mode = pick::<String>(ctx.raw_input, "edit_mode").filter(|s| !s.is_empty());
        let new_source = pick::<String>(ctx.raw_input, "new_source").filter(|s| !s.is_empty());

        let title = match path.as_deref() {
            Some(p) => format!("notebook · {}", p),
            None => "notebook".to_string(),
        };

        // Fence language: read cell_type BEFORE moving it into fields.
        let lang = match cell_type.as_deref() {
            Some("markdown") => "markdown",
            _ => "python",
        };

        let mut fields: Vec<ToolField> = Vec::new();
        if let Some(c) = cell_id {
            fields.push(ToolField {
                label: "cell".to_string(),
                value: c,
            });
        }
        if let Some(t) = cell_type {
            fields.push(ToolField {
                label: "type".to_string(),
                value: t,
            });
        }
        if let Some(m) = edit_mode {
            fields.push(ToolField {
                label: "mode".to_string(),
                value: m,
            });
        }

        // New cell source as a fenced block so MarkdownBody renders
        // it Shiki-highlighted. Defaults to python; markdown cells
        // get a markdown fence (which still preserves layout inside
        // the body).
        let description = new_source.map(|src| format!("```{}\n{}\n```", lang, src));

        let output_text = text_blocks(ctx.content);
        let output = if output_text.is_empty() {
            None
        } else {
            Some(output_text)
        };

        FormattedToolCall {
            title,
            stats: Vec::new(),
            description,
            diff: None,
            output,
            fields,
        }
    }
}

pub fn register(reg: &mut FormatterRegistry, adapter: &str) {
    reg.register_adapter(adapter, "NotebookEdit", Box::new(NotebookEditFormatter));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx<'a>(raw: &'a serde_json::Value, content: &'a [serde_json::Value]) -> FormatterContext<'a> {
        FormatterContext {
            wire_name: "NotebookEdit",
            tool_kind: &crate::tools::ToolKind::Edit,
            raw_input: Some(raw),
            adapter: "acp-claude-code",
            content,
            started_at: 0,
            completed_at: None,
        }
    }

    #[test]
    fn notebook_edit_populates_description_with_python_fence() {
        let raw = json!({
            "notebook_path": "/tmp/nb.ipynb",
            "cell_id": "c-1",
            "cell_type": "code",
            "edit_mode": "replace",
            "new_source": "import numpy as np\nprint(42)",
        });
        let content: Vec<serde_json::Value> = Vec::new();
        let out = NotebookEditFormatter.format(&ctx(&raw, &content));

        // Path is forwarded verbatim — the frontend handles any
        // ellipsing for display. Assert exact match.
        assert_eq!(out.title, "notebook · /tmp/nb.ipynb");
        let desc = out.description.as_deref().expect("description must be populated");

        assert!(desc.starts_with("```python\n"));
        assert!(desc.contains("import numpy"));

        let labels: Vec<&str> = out.fields.iter().map(|f| f.label.as_str()).collect();
        assert!(labels.contains(&"cell"));
        assert!(labels.contains(&"type"));
        assert!(labels.contains(&"mode"));
    }

    #[test]
    fn notebook_edit_markdown_cell_uses_markdown_fence() {
        let raw = json!({
            "notebook_path": "/tmp/nb.ipynb",
            "cell_type": "markdown",
            "new_source": "# Heading",
        });
        let content: Vec<serde_json::Value> = Vec::new();
        let out = NotebookEditFormatter.format(&ctx(&raw, &content));
        let desc = out.description.unwrap();

        assert!(desc.starts_with("```markdown\n"));
    }
}
