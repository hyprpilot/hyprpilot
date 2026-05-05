//! codex-acp's `View Image <path>`. Leading token dispatch is `View`.
//! Fires once at Completed status; rawInput is empty (codex doesn't
//! attach one). The image lands as a `ResourceLink` in the content
//! blocks. Title carries the path; we trim the `View Image ` prefix
//! to surface the path as a field.

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::short_path;
use crate::tools::formatter::types::{FormattedToolCall, ToolField};

pub struct ViewFormatter;

impl ToolFormatter for ViewFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let path = ctx.wire_name.strip_prefix("View Image ").map(str::to_string);
        let title = match path.as_deref() {
            Some(p) => format!("view · {}", short_path(p)),
            None => ctx.wire_name.trim().to_string(),
        };

        let fields = path
            .map(|p| {
                vec![ToolField {
                    label: "path".into(),
                    value: p,
                }]
            })
            .unwrap_or_default();

        FormattedToolCall {
            title,
            stats: Vec::new(),
            description: None,
            output: None,
            fields,
        }
    }
}
