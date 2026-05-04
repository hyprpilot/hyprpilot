use crate::tools::formatter::registry::{FormatterContext, FormatterRegistry, ToolFormatter};
use crate::tools::formatter::shared::{pick, text_blocks};
use crate::tools::formatter::types::FormattedToolCall;

fn host_of(url: &str) -> String {
    if let Some(after_scheme) = url.split_once("://").map(|(_, rest)| rest) {
        let host = after_scheme.split('/').next().unwrap_or(after_scheme);
        host.to_string()
    } else {
        url.to_string()
    }
}

fn sniff_lang(body: &str) -> Option<&'static str> {
    let trimmed = body.trim_start();
    if trimmed.is_empty() {
        return None;
    }
    let head: String = trimmed.chars().take(1024).collect();
    if head.starts_with('{') || head.starts_with('[') {
        return Some("json");
    }
    if head.starts_with("<!DOCTYPE") || head.starts_with("<html") || head.starts_with("<HTML") {
        return Some("html");
    }
    if head.starts_with("<?xml") || head.starts_with("<rss") {
        return Some("xml");
    }
    if head.starts_with("# ") || head.starts_with("## ") || head.contains("\n# ") || head.contains("\n## ") {
        return Some("markdown");
    }
    None
}

pub struct WebFetchFormatter;

impl ToolFormatter for WebFetchFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let target = pick::<String>(ctx.raw_input, "url");
        let prompt = pick::<String>(ctx.raw_input, "prompt").filter(|s| !s.is_empty());

        let host = target.as_deref().map(host_of);
        let title = match (host.as_deref(), prompt.as_deref()) {
            (Some(h), Some(p)) => format!("fetch · {} — {}", h, p),
            (Some(h), None) => format!("fetch · {}", h),
            _ => "fetch".to_string(),
        };

        let body = text_blocks(ctx.content);
        let (description, output) = if body.is_empty() {
            (None, None)
        } else if let Some(lang) = sniff_lang(&body) {
            (Some(format!("```{}\n{}\n```", lang, body)), None)
        } else {
            (None, Some(body))
        };

        FormattedToolCall {
            title,
            stat: None,
            description,
            output,
            fields: Vec::new(),
        }
    }
}

pub fn register(reg: &mut FormatterRegistry, adapter: &str) {
    reg.register_adapter(adapter, "WebFetch", Box::new(WebFetchFormatter));
}
