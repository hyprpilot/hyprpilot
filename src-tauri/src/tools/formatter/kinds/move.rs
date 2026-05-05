//! `move` — file moves / renames. RawInput conventions vary:
//! `source` + `destination`, `from` + `to`, `old_path` + `new_path`.
//! Try each pair in order; the first hit wins.

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::{args_to_fields, pick, wire_title_or_fallback};
use crate::tools::formatter::types::{FormattedToolCall, ToolField};

pub struct MoveFormatter;

impl ToolFormatter for MoveFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let (from, to) = pick_pair(ctx.raw_input);

        let title = wire_title_or_fallback(ctx.wire_name, "move");

        let description = pick::<String>(ctx.raw_input, "description").filter(|s| !s.is_empty());

        let mut fields = args_to_fields(
            ctx.raw_input,
            &[
                "source",
                "destination",
                "from",
                "to",
                "old_path",
                "new_path",
                "description",
            ],
        );
        if let Some(a) = from {
            fields.insert(
                0,
                ToolField {
                    label: "from".into(),
                    value: a,
                },
            );
        }
        if let Some(b) = to {
            fields.insert(
                fields.len().min(1),
                ToolField {
                    label: "to".into(),
                    value: b,
                },
            );
        }

        FormattedToolCall {
            title,
            stat: None,
            description,
            output: None,
            fields,
        }
    }
}

fn pick_pair(raw: Option<&serde_json::Value>) -> (Option<String>, Option<String>) {
    let pairs = [("source", "destination"), ("from", "to"), ("old_path", "new_path")];
    for (a, b) in pairs {
        let from = pick::<String>(raw, a).filter(|s| !s.is_empty());
        let to = pick::<String>(raw, b).filter(|s| !s.is_empty());
        if from.is_some() || to.is_some() {
            return (from, to);
        }
    }
    (None, None)
}
