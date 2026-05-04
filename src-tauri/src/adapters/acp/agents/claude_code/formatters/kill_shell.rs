use crate::tools::formatter::registry::{FormatterContext, FormatterRegistry, ToolFormatter};
use crate::tools::formatter::shared::pick;
use crate::tools::formatter::types::FormattedToolCall;

pub struct KillShellFormatter;

impl ToolFormatter for KillShellFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let id = pick::<String>(ctx.raw_input, "shell_id").or_else(|| pick(ctx.raw_input, "bash_id"));
        let title = match id {
            Some(i) => format!("kill shell #{}", i),
            None => "kill shell".to_string(),
        };

        FormattedToolCall {
            title,
            stat: None,
            description: None,
            output: None,
            fields: Vec::new(),
        }
    }
}

pub fn register(reg: &mut FormatterRegistry, adapter: &str) {
    reg.register_adapter(adapter, "KillShell", Box::new(KillShellFormatter));
}
