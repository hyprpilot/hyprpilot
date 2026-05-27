//! opencode scout repository tools.

use crate::tools::formatter::registry::{FormatterContext, ToolFormatter};
use crate::tools::formatter::shared::{args_to_fields, dedupe_output, duration_stats, pick};
use crate::tools::formatter::types::FormattedToolCall;

pub struct RepoCloneFormatter;

impl ToolFormatter for RepoCloneFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let repository = pick::<String>(ctx.raw_input, "repository").filter(|s| !s.is_empty());
        let title = repository
            .as_deref()
            .map(|repo| format!("repo clone · {repo}"))
            .unwrap_or_else(|| "repo clone".to_string());

        FormattedToolCall {
            title,
            stats: duration_stats(ctx),
            description: None,
            diff: None,
            output: dedupe_output(ctx.content, None),
            fields: args_to_fields(ctx.raw_input, &[]),
        }
    }
}

pub struct RepoOverviewFormatter;

impl ToolFormatter for RepoOverviewFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let path = pick::<String>(ctx.raw_input, "path").filter(|s| !s.is_empty());
        let repository = pick::<String>(ctx.raw_input, "repository").filter(|s| !s.is_empty());
        let target = repository.as_deref().or(path.as_deref());
        let title = target
            .map(|target| format!("repo overview · {target}"))
            .unwrap_or_else(|| "repo overview".to_string());

        FormattedToolCall {
            title,
            stats: duration_stats(ctx),
            description: None,
            diff: None,
            output: dedupe_output(ctx.content, None),
            fields: args_to_fields(ctx.raw_input, &[]),
        }
    }
}
