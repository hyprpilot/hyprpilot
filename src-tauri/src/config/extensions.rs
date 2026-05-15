//! `[[mcps]]` / `[[skills]]` array-of-tables — one extension entry +
//! optional per-entry glob ignore. Both fields use the same matcher
//! the existing MCP tool-name globs (`autoAcceptTools` /
//! `autoRejectTools`) use, so captains have one mental model for
//! "filter X by name pattern" across the config.
//!
//! Glob syntax: `*` (any chars), `?` (single char), `[abc]` (charset).
//! Patterns are anchored against the full slug / server name —
//! `work-*` matches `work-foo` but not `pre-work-foo`. For substring
//! matching write `*-work-*` explicitly.

use std::path::PathBuf;

use garde::Validate;
use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One MCP catalog entry. Two variants share one struct:
///
/// - **File**: `file = "/path/to/servers.json"` — load the standard
///   `{ "mcpServers": { ... } }` JSON shape from disk.
/// - **Inline**: `mcp_servers = { name = { command, args, env, … } }`
///   — same payload the file's `mcpServers` key would carry, declared
///   directly in the hyprpilot config (or in a `--with-config` patch).
///
/// Exactly one of `file` / `mcp_servers` must be set per entry; both
/// or neither fails garde validation at load time.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct McpFile {
    /// Path to a JSON file in the standard
    /// `{ "mcpServers": { ... } }` shape (Claude Code / Codex / Cursor
    /// drop-in compatible). `~` and `$VAR` expand at consume time.
    /// Empty paths fall through to file-read failure with a clear
    /// error at load time; no garde-level check needed.
    #[garde(skip)]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<PathBuf>,
    /// Inline server map — same shape the file's top-level
    /// `mcpServers` key carries. Keys are server names; values stay
    /// as opaque `serde_json::Value` so vendor-specific keys
    /// (`type`, `url`, `headers`, …) pass through to the agent
    /// unchanged. Captains use this for one-off servers under
    /// `--with-config` (e.g. an nvim plugin where one of the env
    /// values has to be set per-invocation).
    ///
    /// `mcp_servers` is mutually exclusive with `file` — see
    /// `validate_mcp_source` for the cross-field check.
    #[garde(custom(validate_mcp_source(&self.file)))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_servers: Option<serde_json::Map<String, Value>>,
    /// Optional glob array. Server names matching ANY pattern are
    /// dropped from the loaded set. Applies uniformly to both file
    /// and inline entries.
    #[garde(custom(validate_globs))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ignore: Option<Vec<String>>,
}

impl McpFile {
    pub fn compile_ignore(&self) -> Option<GlobSet> {
        compile_ignore(self.ignore.as_deref())
    }
}

/// Cross-field invariant for `McpFile`: exactly one of `file` /
/// `mcp_servers` must be set. Attached to `mcp_servers` via garde's
/// self-access pattern (mirrors `validate_agent_default_id`).
fn validate_mcp_source<'a>(
    file: &'a Option<PathBuf>,
) -> impl FnOnce(&Option<serde_json::Map<String, Value>>, &()) -> garde::Result + 'a {
    move |mcp_servers, _ctx| match (file.as_ref(), mcp_servers.as_ref()) {
        (Some(_), Some(_)) => Err(garde::Error::new(
            "mcps entry: set exactly one of `file` or `mcp_servers`, not both",
        )),
        (None, None) => Err(garde::Error::new(
            "mcps entry: must set either `file` (path) or `mcp_servers` (inline map)",
        )),
        _ => Ok(()),
    }
}

/// One skills catalog root entry. Matches both the global `[[skills]]`
/// array and the per-profile override.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct SkillEntry {
    /// Path to a directory containing `<slug>/SKILL.md` bundles.
    /// `~` and `$VAR` expand at consume time.
    /// Empty paths fall through to "directory not found" warnings at
    /// load time; no garde-level check needed.
    #[garde(skip)]
    pub dir: PathBuf,
    /// Optional glob array. Skill slugs matching ANY pattern are
    /// dropped from the loaded set.
    #[garde(custom(validate_globs))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ignore: Option<Vec<String>>,
}

impl SkillEntry {
    pub fn compile_ignore(&self) -> Option<GlobSet> {
        compile_ignore(self.ignore.as_deref())
    }
}

fn compile_ignore(patterns: Option<&[String]>) -> Option<GlobSet> {
    let patterns = patterns?;
    if patterns.is_empty() {
        return None;
    }
    let mut builder = GlobSetBuilder::new();
    for p in patterns {
        if let Ok(glob) = Glob::new(p) {
            builder.add(glob);
        }
    }
    builder.build().ok()
}

fn validate_globs(patterns: &Option<Vec<String>>, _: &()) -> garde::Result {
    let Some(patterns) = patterns else {
        return Ok(());
    };
    for p in patterns {
        if let Err(err) = Glob::new(p) {
            return Err(garde::Error::new(format!(
                "ignore glob '{p}' is not a valid glob pattern: {err}"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mcp_file(path: &str, ignore: Option<Vec<String>>) -> McpFile {
        McpFile {
            file: Some(path.into()),
            mcp_servers: None,
            ignore,
        }
    }

    fn mcp_inline(servers: serde_json::Map<String, serde_json::Value>, ignore: Option<Vec<String>>) -> McpFile {
        McpFile {
            file: None,
            mcp_servers: Some(servers),
            ignore,
        }
    }

    #[test]
    fn validates_well_formed_globs() {
        let f = mcp_file(
            "/tmp/x.json",
            Some(vec!["work-*".into(), "*-internal".into(), "exact-name".into()]),
        );
        f.validate().expect("globs must validate");
    }

    #[test]
    fn rejects_malformed_glob() {
        let f = mcp_file("/tmp/x.json", Some(vec!["[unterminated".into()]));
        let err = f.validate().expect_err("malformed glob must reject");
        assert!(err.to_string().contains("not a valid glob"), "got: {err}");
    }

    #[test]
    fn compile_ignore_matches_glob_patterns() {
        let f = mcp_file("/tmp/x.json", Some(vec!["*-work".into(), "scratch-*".into()]));
        let set = f.compile_ignore().expect("non-empty ignore compiles");
        assert!(set.is_match("linear-laravel-work"));
        assert!(set.is_match("scratch-pad"));
        assert!(!set.is_match("github"));
        assert!(!set.is_match("work-stuff")); // anchored — not "starts-with-work"
    }

    #[test]
    fn compile_ignore_empty_returns_none() {
        let none_case = mcp_file("/tmp/x.json", None);
        assert!(none_case.compile_ignore().is_none());

        let empty_case = mcp_file("/tmp/x.json", Some(vec![]));
        assert!(empty_case.compile_ignore().is_none());
    }

    /// Inline shape passes garde — `mcp_servers` set, `file` unset.
    #[test]
    fn inline_only_validates() {
        let mut servers = serde_json::Map::new();
        servers.insert("alpha".into(), serde_json::json!({ "command": "echo" }));
        let f = mcp_inline(servers, None);
        f.validate().expect("inline shape must validate");
    }

    /// Both `file` and `mcp_servers` set → garde error with a hint
    /// at which knob to drop. Pins the cross-field invariant.
    #[test]
    fn both_file_and_inline_rejects() {
        let mut servers = serde_json::Map::new();
        servers.insert("alpha".into(), serde_json::json!({ "command": "echo" }));
        let f = McpFile {
            file: Some("/tmp/x.json".into()),
            mcp_servers: Some(servers),
            ignore: None,
        };
        let err = f.validate().expect_err("both fields set must reject");
        assert!(err.to_string().contains("exactly one"), "got: {err}");
    }

    /// Neither field set → garde error pointing the captain at the
    /// two legal knobs. Pins the same invariant from the other side.
    #[test]
    fn neither_field_rejects() {
        let f = McpFile::default();
        let err = f.validate().expect_err("empty entry must reject");
        assert!(err.to_string().contains("must set either"), "got: {err}");
    }

    #[test]
    fn skill_entry_uses_same_matcher() {
        let s = SkillEntry {
            dir: "/tmp/skills".into(),
            ignore: Some(vec!["work-*".into()]),
        };
        let set = s.compile_ignore().expect("compiles");
        assert!(set.is_match("work-internal"));
        assert!(!set.is_match("github-pr"));
    }
}
