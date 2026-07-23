//! `system_prompt` array-of-tables — the captain-supplied list of
//! prompt files plus per-entry inject-strategy knobs. Mirrors the
//! `[[mcps]]` / `[[skills]]` shape so all three "list of files +
//! per-entry config" surfaces read identically.
//!
//! ```toml
//! system_prompt = [
//!   { file = "~/.config/hyprpilot/prompts/base.md" },
//!   { file = "~/.config/hyprpilot/prompts/strict.md",
//!     inject = { on_create = true, on_update = true } },
//! ]
//! ```
//!
//! Per-entry `inject` toggles let the captain mix layers — a base
//! persona that only loads on fresh sessions, a strict mode prompt
//! that loads on fresh + restored/forked sessions.
//!
//! Defaults: `on_create = true`, `on_update = false`. Restoring or
//! forking an agent session already carries the previous session's
//! context; re-injecting the system prompt on top is usually noise.
//! Captains opt IN explicitly when they want the prompt to ride on
//! every update bootstrap too.

use std::path::PathBuf;

use garde::Validate;
use serde::{Deserialize, Serialize};

/// One `system_prompt` entry — file path + per-entry inject toggles.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct SystemPromptEntry {
    /// Markdown / text file. `~` + env-var expansion happens at
    /// read time. Required — empty paths reject at validation.
    #[garde(skip)]
    pub file: PathBuf,
    /// Per-entry inject toggles. When omitted: `on_create=true`,
    /// `on_update=false`. Each field independently defaults so
    /// `inject = { on_update = true }` keeps `on_create=true`.
    #[garde(skip)]
    #[serde(default)]
    pub inject: SystemPromptInject,
}

/// Per-injection-path toggles. `on_create` gates injection on a fresh
/// launch (the only path the launcher runs); `on_update` gates
/// injection on a resume/fork path, retained for consumers that
/// distinguish the two.
///
/// Defaults: `on_create = true`, `on_update = false`. A resumed
/// session already carries its transcript context; re-injecting the
/// system prompt on top is usually redundant noise. Opt in explicitly
/// when you want the prompt to ride on every launch.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct SystemPromptInject {
    pub on_create: bool,
    pub on_update: bool,
}

impl Default for SystemPromptInject {
    fn default() -> Self {
        Self {
            on_create: true,
            on_update: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_inject_on_create_only() {
        let inject = SystemPromptInject::default();
        assert!(inject.on_create, "fresh-session default is true");
        assert!(!inject.on_update, "resume default is false (avoid double-inject)");
    }

    #[test]
    fn parses_entry_with_full_inject() {
        let s = r#"
file = "~/.config/hyprpilot/prompts/base.md"

[inject]
on_create = false
on_update = true
"#;
        let entry: SystemPromptEntry = toml::from_str(s).expect("parses");
        assert_eq!(entry.file, PathBuf::from("~/.config/hyprpilot/prompts/base.md"));
        assert!(!entry.inject.on_create);
        assert!(entry.inject.on_update);
    }

    #[test]
    fn parses_entry_with_partial_inject_block() {
        // Only on_update set; on_create falls back to default (true).
        let s = r#"
file = "~/p.md"

[inject]
on_update = true
"#;
        let entry: SystemPromptEntry = toml::from_str(s).expect("parses");
        assert!(entry.inject.on_create, "on_create defaults to true when omitted");
        assert!(entry.inject.on_update);
    }

    #[test]
    fn parses_entry_with_inject_block_omitted() {
        let s = r#"file = "~/p.md""#;
        let entry: SystemPromptEntry = toml::from_str(s).expect("parses");
        assert!(entry.inject.on_create);
        assert!(!entry.inject.on_update);
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

    #[test]
    fn rejects_unknown_field_on_inject() {
        let s = r#"
file = "~/p.md"

[inject]
on_destroy = true
"#;
        let err = toml::from_str::<SystemPromptEntry>(s).expect_err("unknown field");
        assert!(err.to_string().contains("on_destroy"), "{err}");
    }
}
