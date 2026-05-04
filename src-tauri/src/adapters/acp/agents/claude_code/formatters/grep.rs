use serde_json::Value;

use crate::tools::formatter::registry::{FormatterContext, FormatterRegistry, ToolFormatter};
use crate::tools::formatter::shared::{pick, short_path, text_blocks};
use crate::tools::formatter::types::FormattedToolCall;

pub struct GrepFormatter;

impl ToolFormatter for GrepFormatter {
    fn format(&self, ctx: &FormatterContext) -> FormattedToolCall {
        let raw = ctx.raw_input;
        let pattern = pick::<String>(raw, "pattern").filter(|s| !s.is_empty());
        let path = pick::<String>(raw, "path").unwrap_or_else(|| ".".to_string());
        let glob = pick::<String>(raw, "glob");
        let typ = pick::<String>(raw, "type").filter(|s| !s.is_empty());
        let output_mode = pick::<String>(raw, "output_mode").filter(|s| !s.is_empty());

        let mut bits: Vec<String> = vec![format!("in {}", short_path(&path))];
        if let Some(g) = glob {
            bits.push(format!("glob={}", g));
        }
        if let Some(t) = typ {
            bits.push(format!("type={}", t));
        }
        if let Some(m) = output_mode {
            bits.push(format!("mode={}", m));
        }
        if let Some(obj) = raw.and_then(Value::as_object) {
            if matches!(obj.get("-i"), Some(Value::Bool(true))) {
                bits.push("-i".to_string());
            }
            if matches!(obj.get("-n"), Some(Value::Bool(true))) {
                bits.push("-n".to_string());
            }
        }

        let title = match pattern {
            Some(p) => format!("grep · {} · {}", p, bits.join(" ")),
            None => "grep".to_string(),
        };
        let body = text_blocks(ctx.content);
        let output = if body.is_empty() { None } else { Some(body) };

        FormattedToolCall {
            title,
            stat: None,
            description: None,
            output,
            fields: Vec::new(),
        }
    }
}

pub fn register(reg: &mut FormatterRegistry, adapter: &str) {
    reg.register_adapter(adapter, "Grep", Box::new(GrepFormatter));
}
