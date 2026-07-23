//! `system_prompt` array-of-tables — the captain-supplied list of
//! prompt files plus a per-entry inject toggle. Mirrors the
//! `[[mcps]]` / `[[skills]]` shape so all three "list of files +
//! per-entry config" surfaces read identically.
//!
//! ```toml
//! system_prompt = [
//!   { file = "~/.config/hyprpilot/prompts/base.md" },
//!   { file = "~/.config/hyprpilot/prompts/strict.md", inject = false },
//! ]
//! ```
//!
//! Per-entry `inject` lets the captain mix layers — a base persona
//! that always loads, an addendum kept in the file for reference but
//! excluded from injection.
//!
//! Default: `inject = true`. hyprpilot is fire-and-exec — every
//! launch is a fresh session, so there is no resume/update path to
//! gate separately; a single on/off toggle is the whole surface.

use std::path::PathBuf;

use garde::Validate;
use serde::{Deserialize, Serialize};

fn default_inject() -> bool {
    true
}

/// One `system_prompt` entry — file path + the per-entry inject
/// toggle.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct SystemPromptEntry {
    /// Markdown / text file. `~` + env-var expansion happens at
    /// read time; a missing/unreadable file surfaces then, not at
    /// validation (the path is `#[garde(skip)]` — not shape-checked).
    #[garde(skip)]
    pub file: PathBuf,
    /// Whether this entry's body rides the launch's system-prompt
    /// injection. Defaults to `true`; set `false` to keep a file in
    /// the list (e.g. for documentation/reference) without it
    /// actually being injected.
    #[garde(skip)]
    #[serde(default = "default_inject")]
    pub inject: bool,
}

impl Default for SystemPromptEntry {
    fn default() -> Self {
        Self {
            file: PathBuf::new(),
            inject: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_inject_true() {
        assert!(
            SystemPromptEntry::default().inject,
            "fire-and-exec default is inject-on"
        );
    }

    #[test]
    fn parses_entry_with_inject_true() {
        let s = r#"
file = "~/.config/hyprpilot/prompts/base.md"
inject = true
"#;
        let entry: SystemPromptEntry = toml::from_str(s).expect("parses");
        assert_eq!(entry.file, PathBuf::from("~/.config/hyprpilot/prompts/base.md"));
        assert!(entry.inject);
    }

    #[test]
    fn parses_entry_with_inject_false() {
        let s = r#"
file = "~/p.md"
inject = false
"#;
        let entry: SystemPromptEntry = toml::from_str(s).expect("parses");
        assert!(!entry.inject);
    }

    #[test]
    fn parses_entry_with_inject_omitted_defaults_true() {
        let s = r#"file = "~/p.md""#;
        let entry: SystemPromptEntry = toml::from_str(s).expect("parses");
        assert!(entry.inject);
    }

    #[test]
    fn rejects_unknown_field_on_entry() {
        let s = r#"
file = "~/p.md"
bogus = "x"
"#;
        let err = toml::from_str::<SystemPromptEntry>(s).expect_err("unknown field");
        assert!(err.to_string().contains("bogus"), "{err}");
    }
}
