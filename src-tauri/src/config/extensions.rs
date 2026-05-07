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

/// One MCP catalog file entry. Matches both the global `[[mcps]]`
/// array and the per-profile override.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct McpFile {
    /// Path to a JSON file in the standard
    /// `{ "mcpServers": { ... } }` shape (Claude Code / Codex / Cursor
    /// drop-in compatible). `~` and `$VAR` expand at consume time.
    /// Empty paths fall through to file-read failure with a clear
    /// error at load time; no garde-level check needed.
    #[garde(skip)]
    pub file: PathBuf,
    /// Optional glob array. Server names matching ANY pattern are
    /// dropped from the loaded set.
    #[garde(custom(validate_globs))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ignore: Option<Vec<String>>,
}

impl McpFile {
    pub fn compile_ignore(&self) -> Option<GlobSet> {
        compile_ignore(self.ignore.as_deref())
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

    #[test]
    fn validates_well_formed_globs() {
        let f = McpFile {
            file: "/tmp/x.json".into(),
            ignore: Some(vec!["work-*".into(), "*-internal".into(), "exact-name".into()]),
        };
        f.validate().expect("globs must validate");
    }

    #[test]
    fn rejects_malformed_glob() {
        let f = McpFile {
            file: "/tmp/x.json".into(),
            ignore: Some(vec!["[unterminated".into()]),
        };
        let err = f.validate().expect_err("malformed glob must reject");
        assert!(err.to_string().contains("not a valid glob"), "got: {err}");
    }

    #[test]
    fn compile_ignore_matches_glob_patterns() {
        let f = McpFile {
            file: "/tmp/x.json".into(),
            ignore: Some(vec!["*-work".into(), "scratch-*".into()]),
        };
        let set = f.compile_ignore().expect("non-empty ignore compiles");
        assert!(set.is_match("linear-laravel-work"));
        assert!(set.is_match("scratch-pad"));
        assert!(!set.is_match("github"));
        assert!(!set.is_match("work-stuff")); // anchored — not "starts-with-work"
    }

    #[test]
    fn compile_ignore_empty_returns_none() {
        let none_case = McpFile {
            file: "/tmp/x.json".into(),
            ignore: None,
        };
        assert!(none_case.compile_ignore().is_none());

        let empty_case = McpFile {
            file: "/tmp/x.json".into(),
            ignore: Some(vec![]),
        };
        assert!(empty_case.compile_ignore().is_none());
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
