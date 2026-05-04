//! Cross-formatter primitives. Pure helpers; no Tauri / no I/O.

use convert_case::{Case, Casing};
use serde_json::Value;

use crate::tools::formatter::types::ToolField;

/// Project a rawInput key (camelCase / snake_case / PascalCase /
/// SCREAMING_SNAKE) onto the human label the spec sheet renders:
/// space-separated lowercase words. The CSS layer applies its own
/// `text-transform: uppercase`, so the captain reads
/// `planFilepath` → `plan filepath` → `PLAN FILEPATH` instead of the
/// jammed-together `PLANFILEPATH`. Empty input passes through.
pub fn human_label(key: &str) -> String {
    if key.is_empty() {
        return String::new();
    }
    key.to_case(Case::Lower)
}

/// Pick a typed arg straight off the agent's raw `tool_call.rawInput`.
/// `None` for missing args, missing keys, or `from_value` failures.
/// `T` is anything `serde_json` can deserialise — `String`, `i64`,
/// `bool`, `Vec<Value>`, custom structs.
///
/// Per-vendor formatters reach for the exact wire-key the vendor
/// emits (`file_path` for claude-code's `Read`, `bash_id` for `Bash`,
/// etc.). No name normalisation — each formatter knows its vendor's
/// arg shape.
///
/// Callers filter "useful" values themselves where the semantics
/// matter (`pick::<String>(raw, "path").filter(|s| !s.is_empty())`).
pub fn pick<T: serde::de::DeserializeOwned>(args: Option<&Value>, key: &str) -> Option<T> {
    args?.get(key).cloned().and_then(|v| serde_json::from_value(v).ok())
}

/// Project an arg map onto structured `ToolField` rows. Used by the
/// generic MCP formatter and the `other` fallback. Values render as
/// code blocks in the spec sheet; nested objects fall back to
/// JSON-stringified one-liners. `exclude` skips keys routed
/// elsewhere on the view (canonical case: `description` extracts to
/// the view's `description` field).
pub fn args_to_fields(raw: Option<&Value>, exclude: &[&str]) -> Vec<ToolField> {
    let map = match raw.and_then(|v| v.as_object()) {
        Some(m) => m,
        None => return Vec::new(),
    };
    let mut out = Vec::with_capacity(map.len());

    for (k, v) in map {
        if exclude.contains(&k.as_str()) {
            continue;
        }

        if v.is_null() {
            continue;
        }
        let value = match v {
            Value::String(s) if !s.is_empty() => s.clone(),
            Value::String(_) => continue,
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => n.to_string(),
            other => match serde_json::to_string(other) {
                Ok(s) => s,
                Err(_) => continue,
            },
        };

        out.push(ToolField {
            label: human_label(k),
            value,
        });
    }
    out
}

/// `text_blocks` projection that drops the result when it matches
/// the LLM-supplied `description` arg verbatim. Some agents echo
/// `rawInput.description` into the initial `tool_call.content` as a
/// preview; without this dedupe the same prose renders twice — once
/// in the formatted `description`, once in the `output` block.
pub fn dedupe_output(content: &[Value], description: Option<&str>) -> Option<String> {
    let text = text_blocks(content);
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let desc = description.map(str::trim).unwrap_or("");
    if !desc.is_empty() && trimmed == desc {
        return None;
    }
    Some(trimmed.to_string())
}

/// Joined text from every wire content block. Handles both ACP shapes:
/// the bare `{"type":"text","text":"..."}` form (some adapters emit it
/// directly as `tool_call.content`) and the spec-compliant
/// `{"type":"content","content":{"type":"text","text":"..."}}` envelope
/// (`ToolCallContent::Content`). Non-text variants (`image` / `audio` /
/// `resource_link` / `resource`) skip — bash / read / web_fetch
/// formatters only care about prose output here.
pub fn text_blocks(content: &[Value]) -> String {
    let mut parts: Vec<String> = Vec::new();

    for block in content {
        let inner = match block.get("type").and_then(Value::as_str) {
            Some("content") => block.get("content"),
            Some("text") => Some(block),
            _ => None,
        };
        let Some(inner) = inner else { continue };
        if inner.get("type").and_then(Value::as_str) != Some("text") {
            continue;
        }
        if let Some(text) = inner.get("text").and_then(Value::as_str) {
            if !text.is_empty() {
                parts.push(text.to_string());
            }
        }
    }
    parts.join("\n\n")
}

/// Title prefix derived from `ctx.wire_name`. Returns the first
/// whitespace-delimited token lowercased so kind-default formatters
/// preserve the tool's own identity in the rendered title (`bash`
/// stays `bash`, not the kind verb `execute`). Falls back to
/// `fallback` when wire_name is empty.
pub fn title_prefix(wire_name: &str, fallback: &str) -> String {
    let trimmed = wire_name.trim();
    if trimmed.is_empty() {
        return fallback.to_string();
    }
    trimmed
        .split_whitespace()
        .next()
        .unwrap_or(fallback)
        .to_lowercase()
}

/// Project an `(old_text, new_text)` pair onto a Shiki-friendly diff
/// markdown block. Two-tier strategy:
///
/// - **Rich (per-language)**: when `path` resolves to a language with
///   a known line-comment style (`//`, `#`, `--`), every old line
///   becomes `<line> <comment> [!code --]` and every new line
///   becomes `<line> <comment> [!code ++]`. UI's `MarkdownBody` runs
///   `transformerNotationDiff` which strips the markers + adds
///   `.line.diff.{add,remove}` CSS classes — captain reads full
///   per-language syntax highlighting WITH diff coloring.
/// - **Cheap (`diff` fence)**: path-less calls or unknown extensions
///   fall through to a `\`\`\`diff` fence with `+`/`-` line prefixes.
///   Shiki's built-in `diff` grammar colors red/green; no per-token
///   highlighting. Always-correct fallback.
///
/// Both old and new empty → `None` so the caller drops it.
pub fn format_diff_hunk(path: Option<&str>, old_text: &str, new_text: &str) -> Option<String> {
    if old_text.is_empty() && new_text.is_empty() {
        return None;
    }
    let lang = path.and_then(lang_from_path);
    let comment = lang.and_then(comment_for_lang);
    match (lang, comment) {
        (Some(lang), Some(comment)) => Some(rich_diff_hunk(lang, comment, old_text, new_text)),
        _ => Some(cheap_diff_hunk(old_text, new_text)),
    }
}

fn rich_diff_hunk(lang: &str, comment: &str, old_text: &str, new_text: &str) -> String {
    let mut out = format!("```{}\n", lang);
    for line in old_text.lines() {
        out.push_str(line);
        out.push(' ');
        out.push_str(comment);
        out.push_str(" [!code --]\n");
    }
    for line in new_text.lines() {
        out.push_str(line);
        out.push(' ');
        out.push_str(comment);
        out.push_str(" [!code ++]\n");
    }
    out.push_str("```");
    out
}

fn cheap_diff_hunk(old_text: &str, new_text: &str) -> String {
    let mut out = String::from("```diff\n");
    for line in old_text.lines() {
        out.push_str("- ");
        out.push_str(line);
        out.push('\n');
    }
    for line in new_text.lines() {
        out.push_str("+ ");
        out.push_str(line);
        out.push('\n');
    }
    out.push_str("```");
    out
}

/// Comment style for `transformerNotationDiff` markers. Languages
/// where end-of-line line-comments don't exist (`html` / `xml` /
/// `markdown` / `plaintext`) return `None`; the caller then falls
/// back to the cheap `\`\`\`diff` fence.
pub fn comment_for_lang(lang: &str) -> Option<&'static str> {
    Some(match lang {
        "typescript" | "javascript" | "rust" | "go" | "java" | "kotlin" | "swift" | "csharp" | "cpp" | "c" | "css"
        | "scss" | "json" | "vue" => "//",
        "python" | "bash" | "yaml" | "toml" | "ruby" => "#",
        "lua" | "sql" => "--",
        _ => return None,
    })
}

/// Trim a long path to its last two segments so the title stays
/// narrow. `.../<parent>/<leaf>` for paths with 3+ segments.
pub fn short_path(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    if parts.len() <= 2 {
        return path.to_string();
    }

    format!(".../{}/{}", parts[parts.len() - 2], parts[parts.len() - 1])
}

/// `mcp__server__leaf` parsing. Empty server / leaf returns `None`.
pub struct McpName<'a> {
    pub server: &'a str,
    pub leaf: String,
}

pub fn parse_mcp(canonical: &str) -> Option<McpName<'_>> {
    if !canonical.starts_with("mcp__") {
        return None;
    }
    let parts: Vec<&str> = canonical.split("__").collect();
    if parts.len() < 3 {
        return None;
    }
    let server = parts[1];
    let leaf = parts[2..].join("__");
    if server.is_empty() || leaf.is_empty() {
        return None;
    }
    Some(McpName { server, leaf })
}

/// Path → fenced-code language hint. Mirrors the TS
/// `inferMimeFromPath`+`resolveShikiLang` chain — this returns the
/// Shiki language name directly (skipping the MIME hop) since the
/// daemon-side formatters consume it for fence labels only.
pub fn lang_from_path(path: &str) -> Option<&'static str> {
    let seg = path.rsplit('/').next()?;
    let dot = seg.rfind('.')?;
    let ext = &seg[dot + 1..].to_ascii_lowercase();

    Some(match ext.as_str() {
        "ts" | "tsx" => "typescript",
        "js" | "jsx" => "javascript",
        "json" => "json",
        "md" => "markdown",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "rs" => "rust",
        "go" => "go",
        "py" => "python",
        "sh" | "bash" | "zsh" => "bash",
        "sql" => "sql",
        "vue" => "vue",
        "html" | "htm" => "html",
        "css" => "css",
        "scss" => "scss",
        "lua" => "lua",
        "rb" => "ruby",
        "java" => "java",
        "kt" => "kotlin",
        "swift" => "swift",
        "c" | "h" => "c",
        "cpp" | "cc" | "hpp" => "cpp",
        "cs" => "csharp",
        _ => return None,
    })
}
