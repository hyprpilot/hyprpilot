//! claude-code's plan-exit tool — the canonical Modal-permission
//! driver. The plan body is markdown; the modal renders it via
//! `<MarkdownBody>` so the captain reviews + accepts before the agent
//! leaves plan mode.
//!
//! claude-code-acp ≥0.32 emits this as `switch_mode` (with a `plan`
//! rawInput); older builds emit `ExitPlanMode`. Register under both
//! names so the dispatch hits regardless of the SDK release.
//! `plan_filepath` is the agent-resolved plan-on-disk path — surfaced
//! as a single field, NOT in the structured args dump that would
//! otherwise duplicate the plan text.

use crate::tools::formatter::registry::{FormatterContext, FormatterRegistry, ToolFormatter};
use crate::tools::formatter::shared::pick;
use crate::tools::formatter::types::{FormattedToolCall, ToolField};

pub struct PlanExitFormatter;

impl ToolFormatter for PlanExitFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let plan = pick::<String>(ctx.raw_input, "plan").filter(|s| !s.is_empty());
        let plan_filepath = pick::<String>(ctx.raw_input, "planFilepath")
            .or_else(|| pick(ctx.raw_input, "plan_filepath"))
            .filter(|s| !s.is_empty());

        let mut fields: Vec<ToolField> = Vec::new();
        if let Some(p) = plan_filepath {
            fields.push(ToolField {
                label: "plan path".into(),
                value: p,
            });
        }

        FormattedToolCall {
            title: "plan ready for review".to_string(),
            stat: None,
            description: plan,
            output: None,
            fields,
        }
    }
}

pub fn register(reg: &mut FormatterRegistry, adapter: &str) {
    reg.register_adapter(adapter, "ExitPlanMode", Box::new(PlanExitFormatter));
    reg.register_adapter(adapter, "switch_mode", Box::new(PlanExitFormatter));
    // claude-code-acp ≥0.32 emits the switch_mode tool with a prose
    // title ("Ready to code?", "EnterPlanMode", varies per direction)
    // — neither the wire-name registration above nor the leading-
    // token tier discriminate. The discriminating signal is a
    // string-shaped `plan` key in rawInput; no other claude-code
    // tool carries one. Sibling keys (`planFilepath` / `plan_filepath`
    // / `allowedPrompts`) vary by SDK build, so we don't anchor on
    // them.
    reg.register_adapter_match(
        adapter,
        |ctx| {
            ctx.raw_input
                .and_then(|v| v.as_object())
                .and_then(|m| m.get("plan"))
                .and_then(|v| v.as_str())
                .is_some_and(|s| !s.is_empty())
        },
        Box::new(PlanExitFormatter),
    );
}
