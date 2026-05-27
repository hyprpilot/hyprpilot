//! Cross-formatter primitives. Pure helpers; no Tauri / no I/O.

use convert_case::{Case, Casing};
use serde_json::Value;

use crate::tools::formatter::registry::FormatterContext;
use crate::tools::formatter::types::{Stat, ToolField};

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

/// `text_blocks` projection that drops the result when it matches the
/// LLM-supplied `description` arg verbatim. The returned output keeps
/// the agent's bytes as-is; trimming is only for empty/duplicate
/// comparisons. Some agents echo `rawInput.description` into the
/// initial `tool_call.content` as a preview; without this dedupe the
/// same prose renders twice — once in the formatted `description`,
/// once in the `output` block.
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
    Some(text)
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

/// Use the agent's wire title verbatim. Trims leading/trailing
/// whitespace; falls back to `fallback` only when empty. Kind-default
/// formatters preserve the agent's voice — `"Edit /tmp/foo"` stays
/// `"Edit /tmp/foo"`, not lowercased to `"edit"`.
pub fn wire_title_or_fallback(wire_name: &str, fallback: &str) -> String {
    let trimmed = wire_name.trim();
    if trimmed.is_empty() {
        return fallback.to_string();
    }
    trimmed.to_string()
}

/// `Stat::Duration` from the cached `started_at` / `completed_at`
/// timestamps on a `FormatterContext`. Returns `None` while the
/// tool is mid-flight (the captain decided no live-tick this MR;
/// the pill stays statless until the call settles). Saturating
/// subtraction so a degenerate `started_at > completed_at` (clock
/// drift, system suspend) doesn't underflow.
pub fn duration_stat(ctx: &FormatterContext) -> Option<Stat> {
    let completed = ctx.completed_at?;
    let ms = completed.saturating_sub(ctx.started_at);
    Some(Stat::Duration { ms })
}

/// `duration_stat(ctx)` projected onto a `Vec<Stat>` ready to drop
/// onto `FormattedToolCall.stats`. Empty when the tool's still
/// mid-flight. Folds the `Option → Vec` boilerplate from the dozen
/// per-vendor formatters that all want exactly one duration pill.
pub fn duration_stats(ctx: &FormatterContext) -> Vec<Stat> {
    duration_stat(ctx).into_iter().collect()
}

/// Count newline-separated lines in `(old, new)` for the
/// `Stat::Diff` pill. Returns `(added, removed)` — both as the
/// magnitude of each side, NOT a true LCS. For an in-place edit we
/// report `(new.lines().count(), old.lines().count())` — the user
/// reads "+12 −3" as size of change, not unchanged-vs-changed
/// accounting. Run a real diff library only if precision actually
/// matters somewhere; the captain's call is "magnitude is enough".
///
/// Empty strings count zero lines (`"".lines()` yields no items).
/// A trailing-newline-only difference (`"a\n"` vs `"a"`) reports
/// `(1, 1)` per `lines()`'s rules — fine for the magnitude reading.
pub fn line_magnitudes(old_text: &str, new_text: &str) -> (u32, u32) {
    let added = new_text.lines().count() as u32;
    let removed = old_text.lines().count() as u32;
    (added, removed)
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

/// Project an `(old_text, new_text)` pair onto a unified git-diff
/// patch string — `--- a/<path>` + `+++ b/<path>` + `@@ -1,N +1,M @@`
/// headers followed by `+/-` lines. Distinct from
/// `format_diff_hunk` (which emits Shiki-marker markdown);
/// consumers that can't run the Shiki transformer pipeline (plain
/// markdown renderers, Neovim, GitHub paste, `patch -p1`-style
/// tooling) read this field instead.
///
/// Hunk offsets are synthesised at `1` because Edit's old/new
/// snippets aren't anchored to source line numbers — the agent
/// supplies free-form text, not a file-offset patch. Consumers that
/// need real anchoring still have `raw_input.old_string` /
/// `new_string` and can compute their own.
///
/// `path = None` skips the file headers and emits just
/// `@@ ... @@` + `+/-` lines (suitable for embedding in markdown
/// without a file identity).
///
/// Both old and new empty → `None` so the caller drops it.
pub fn format_git_diff(path: Option<&str>, old_text: &str, new_text: &str) -> Option<String> {
    if old_text.is_empty() && new_text.is_empty() {
        return None;
    }
    let removed = old_text.lines().count();
    let added = new_text.lines().count();
    let old_start = if removed == 0 { 0 } else { 1 };
    let new_start = if added == 0 { 0 } else { 1 };

    let mut out = String::new();
    if let Some(p) = path.filter(|s| !s.is_empty()) {
        let p = p.trim_start_matches('/');
        out.push_str("diff --git a/");
        out.push_str(p);
        out.push_str(" b/");
        out.push_str(p);
        out.push('\n');
        out.push_str("--- a/");
        out.push_str(p);
        out.push('\n');
        out.push_str("+++ b/");
        out.push_str(p);
        out.push('\n');
    }
    out.push_str(&format!("@@ -{old_start},{removed} +{new_start},{added} @@\n"));
    for line in old_text.lines() {
        out.push('-');
        out.push_str(line);
        out.push('\n');
    }
    for line in new_text.lines() {
        out.push('+');
        out.push_str(line);
        out.push('\n');
    }
    Some(out)
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn dedupe_output_preserves_visible_whitespace() {
        let content = vec![json!({
            "type": "text",
            "text": "    first line\n  second line\n"
        })];

        assert_eq!(
            dedupe_output(&content, None).as_deref(),
            Some("    first line\n  second line\n")
        );
    }

    #[test]
    fn dedupe_output_trims_for_duplicate_comparison_only() {
        let content = vec![json!({
            "type": "text",
            "text": "  repeated description\n"
        })];

        assert!(dedupe_output(&content, Some("repeated description")).is_none());
    }

    #[test]
    fn line_magnitudes_all_add_when_old_empty() {
        let (added, removed) = line_magnitudes("", "fn foo() {\n    bar()\n}");
        assert_eq!(added, 3);
        assert_eq!(removed, 0);
    }

    #[test]
    fn line_magnitudes_all_remove_when_new_empty() {
        let (added, removed) = line_magnitudes("fn foo() {\n    bar()\n}", "");
        assert_eq!(added, 0);
        assert_eq!(removed, 3);
    }

    #[test]
    fn line_magnitudes_in_place_edit_reports_each_side() {
        // Magnitude reading, not LCS — both lines on each side count
        // as the size of the change.
        let (added, removed) = line_magnitudes("a\nb\nc", "a\nB\nc\nd");
        assert_eq!(added, 4);
        assert_eq!(removed, 3);
    }

    #[test]
    fn line_magnitudes_empty_strings_are_zero() {
        let (added, removed) = line_magnitudes("", "");
        assert_eq!(added, 0);
        assert_eq!(removed, 0);
    }

    #[test]
    fn line_magnitudes_trailing_newline_does_not_inflate() {
        // `"a\n".lines()` yields one item, same as `"a"`. Trailing
        // newlines don't bump the count up — fine for our magnitude
        // reading.
        let (added, _) = line_magnitudes("", "a\n");
        assert_eq!(added, 1);
    }

    #[test]
    fn format_git_diff_with_path_emits_file_headers_and_hunk() {
        let patch = format_git_diff(Some("src/foo.rs"), "let x = 1;", "let x = 2;").expect("patch");
        assert!(patch.starts_with("diff --git a/src/foo.rs b/src/foo.rs\n"), "{patch}");
        assert!(patch.contains("--- a/src/foo.rs\n"), "{patch}");
        assert!(patch.contains("+++ b/src/foo.rs\n"), "{patch}");
        assert!(patch.contains("@@ -1,1 +1,1 @@\n"), "{patch}");
        assert!(patch.contains("-let x = 1;\n"), "{patch}");
        assert!(patch.contains("+let x = 2;\n"), "{patch}");
    }

    #[test]
    fn format_git_diff_strips_leading_slash_from_path() {
        // Git patches use repo-relative paths under `a/` and `b/`;
        // a leading slash on the source would produce `a//abs/path`.
        let patch = format_git_diff(Some("/abs/path.rs"), "old", "new").expect("patch");
        assert!(patch.contains("a/abs/path.rs"), "{patch}");
        assert!(!patch.contains("a//abs/path.rs"), "double slash: {patch}");
    }

    #[test]
    fn format_git_diff_without_path_skips_file_headers() {
        let patch = format_git_diff(None, "old", "new").expect("patch");
        assert!(!patch.contains("diff --git"), "{patch}");
        assert!(!patch.contains("---"), "{patch}");
        assert!(!patch.contains("+++"), "{patch}");
        assert!(patch.starts_with("@@ -1,1 +1,1 @@\n"), "{patch}");
        assert!(patch.contains("-old\n"));
        assert!(patch.contains("+new\n"));
    }

    /// Split a patch into its hunk body (everything after the
    /// `@@ ... @@` header line). Tests use this to assert per-line
    /// `+/-` presence without false-matching on the `--- a/` /
    /// `+++ b/` file headers that also start with `-` / `+`.
    fn hunk_body(patch: &str) -> &str {
        let (_, rest) = patch.split_once("@@\n").expect("hunk header");
        rest
    }

    #[test]
    fn format_git_diff_write_pattern_uses_zero_old_offset() {
        // Empty old + non-empty new = "new file" pattern. Git's
        // convention is `@@ -0,0 +1,M @@`.
        let patch = format_git_diff(Some("new.txt"), "", "first line\nsecond line").expect("patch");
        assert!(patch.contains("@@ -0,0 +1,2 @@\n"), "{patch}");
        let body = hunk_body(&patch);
        assert!(body.contains("+first line\n"));
        assert!(body.contains("+second line\n"));
        // No `-` lines in the hunk body since old is empty.
        for line in body.lines() {
            assert!(!line.starts_with('-'), "spurious removal line: {line}");
        }
    }

    #[test]
    fn format_git_diff_delete_pattern_uses_zero_new_offset() {
        let patch = format_git_diff(Some("gone.txt"), "first\nsecond", "").expect("patch");
        assert!(patch.contains("@@ -1,2 +0,0 @@\n"), "{patch}");
        let body = hunk_body(&patch);
        assert!(body.contains("-first\n"));
        assert!(body.contains("-second\n"));
        for line in body.lines() {
            assert!(!line.starts_with('+'), "spurious addition line: {line}");
        }
    }

    #[test]
    fn format_git_diff_empty_inputs_return_none() {
        assert!(format_git_diff(Some("foo.rs"), "", "").is_none());
        assert!(format_git_diff(None, "", "").is_none());
    }

    #[test]
    fn format_git_diff_orders_removals_before_additions() {
        // The hunk emits all `-` lines first, then all `+` lines —
        // matches the rich/cheap Shiki helpers and what `patch -p1`
        // expects for non-anchored hunks.
        let patch = format_git_diff(Some("a.rs"), "a\nb", "c\nd").expect("patch");
        let minus_a = patch.find("-a").expect("removal line");
        let plus_c = patch.find("+c").expect("addition line");
        assert!(minus_a < plus_c, "removals must precede additions: {patch}");
    }
}
