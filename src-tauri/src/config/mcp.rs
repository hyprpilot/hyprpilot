//! `[mcp]` config block — controls the in-tree `hyprpilot` MCP server
//! the daemon auto-injects into `session/new`'s `mcp_servers`, and
//! owns the **skills catalog** the server exposes.
//!
//! Singleton block mirroring the `[agent]` / `[[agents]]` pattern:
//! `[mcp]` is the global config for our in-tree MCP server; `[[mcps]]`
//! is the captain-declared catalog of *external* MCP servers. Same
//! TOML neighbourhood, distinct concerns.
//!
//! Skills live under `[[mcp.skills]]` (was top-level `[[skills]]`).
//! They belong here because the hyprpilot MCP server is what exposes
//! them to the agent; there is no consumer of skills outside the MCP
//! server.
//!
//! Global vs per-profile: top-level `Config.mcp` is seeded by
//! `defaults.toml`; per-profile `ProfileConfig.mcp: Option<McpConfig>`
//! wholesale-replaces the global when set — mirroring the `mcps`
//! profile-override pattern. Field shapes are `Option<T>` so the
//! `overwrite_some` merge strategy can layer defaults.toml → user
//! config.toml → per-profile cleanly, same as `Autostart`
//! (`src-tauri/src/config/autostart.rs`).

use garde::Validate;
use merge::Merge;
use serde::{Deserialize, Serialize};

use super::merge_strategies::overwrite_some;
use super::SkillEntry;

/// `[mcp]` block. Controls auto-injection of the in-tree
/// `hyprpilot mcp serve` MCP server entry into `session/new` /
/// `session/load`'s `mcp_servers` array, and owns the **skills
/// catalog** that server exposes.
///
/// `auto_accept_tools` / `auto_reject_tools` ride through to the
/// auto-injected entry's `HyprpilotExtension` namespace key so
/// `PermissionController::decide` lane 2 sees them like any
/// user-declared MCP. The default `["*"]` accept makes
/// `mcp__hyprpilot__*` calls frictionless; the captain can tighten
/// per-profile if they want explicit gates around `read_skill` etc.
///
/// `skills` is the **catalog of skill root directories** the server
/// scans and exposes. Same `SkillEntry { dir, ignore }` shape that
/// used to live at the top-level `[[skills]]`. `None` (default) →
/// the daemon falls through to the seeded default
/// (`~/.config/hyprpilot/skills`). `Some([])` → no skills at all
/// (suppresses auto-inject — nothing to serve). `Some([...])` →
/// wholesale-replaces the default catalog.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
#[merge(strategy = overwrite_some)]
pub struct McpConfig {
    /// `true` (default — seeded by defaults.toml) → daemon auto-injects
    /// the in-tree MCP server when `skills` resolves to a non-empty
    /// catalog. `false` → skip auto-inject entirely; agent sees no
    /// `hyprpilot` server. Profile-level `false` override lets the
    /// captain run a vendor-only session.
    #[garde(skip)]
    pub enabled: Option<bool>,

    /// Skill catalog roots. Each entry is a directory of
    /// `<slug>/SKILL.md` bundles plus an optional per-entry glob
    /// `ignore` array filtering slugs at load time. Mirrors the old
    /// top-level `[[skills]]` shape verbatim — only the location
    /// moved.
    #[garde(dive)]
    pub skills: Option<Vec<SkillEntry>>,

    /// Glob patterns matching `mcp__hyprpilot__<tool>` leaf names for
    /// auto-accept. Default `["*"]` (seeded by defaults.toml) → every
    /// `tools/call` against the auto-injected server short-circuits
    /// to `Decision::Allow` at `PermissionController::decide` lane 2.
    /// Tighten by setting `auto_accept_tools = ["list_*", "read_*"]`
    /// (and leaving `reload` to AskUser, say).
    #[garde(skip)]
    pub auto_accept_tools: Option<Vec<String>>,

    /// Glob patterns for auto-reject. Default `[]` (seeded by
    /// defaults.toml) — no rejects, every call falls through to the
    /// accept lane. Reject beats accept on overlap inside the lane.
    #[garde(skip)]
    pub auto_reject_tools: Option<Vec<String>>,
}

impl Default for McpConfig {
    /// Mirror of the values seeded in `defaults.toml`. Lives here too
    /// so `Config::default()` (used by tests that bypass the loader)
    /// gets the same shape the production daemon sees. The
    /// `defaults_seed_mcp_block` test in `config/mod.rs` and the
    /// `default_matches_defaults_toml_seeded_values` test below pin
    /// both representations together — a drift between them fails
    /// the suite before it ships.
    fn default() -> Self {
        Self {
            enabled: Some(true),
            skills: Some(vec![SkillEntry {
                dir: std::path::PathBuf::from("~/.config/hyprpilot/skills"),
                ignore: None,
            }]),
            auto_accept_tools: Some(vec!["*".to_string()]),
            auto_reject_tools: Some(Vec::new()),
        }
    }
}

impl McpConfig {
    /// `enabled.expect("seeded by defaults.toml")` — fatal if the
    /// defaults didn't seed it. The paired test
    /// `defaults_seed_mcp_block` pins every leaf so this never
    /// panics at runtime.
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.enabled.expect("[mcp] enabled seeded by defaults.toml")
    }

    /// Auto-accept globs as a borrowed slice. Defaults to `["*"]` per
    /// defaults.toml.
    #[must_use]
    pub fn auto_accept_tools(&self) -> &[String] {
        self.auto_accept_tools
            .as_deref()
            .expect("[mcp] autoAcceptTools seeded by defaults.toml")
    }

    /// Auto-reject globs as a borrowed slice. Defaults to `[]` per
    /// defaults.toml.
    #[must_use]
    pub fn auto_reject_tools(&self) -> &[String] {
        self.auto_reject_tools
            .as_deref()
            .expect("[mcp] autoRejectTools seeded by defaults.toml")
    }

    /// Resolve every `[[mcp.skills]]` entry to its absolute path +
    /// compiled ignore matcher. `~` / `$VAR` expansion happens here.
    /// Returns an empty vec when `skills` is unset; treat that as
    /// "no skills" semantically.
    #[must_use]
    pub fn resolved_skills(&self) -> Vec<super::ResolvedSkillEntry> {
        self.skills
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .map(|e| super::ResolvedSkillEntry {
                dir: crate::paths::resolve_user(&e.dir.to_string_lossy()),
                ignore: e.compile_ignore(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_matches_defaults_toml_seeded_values() {
        // Pairs with `config::tests::defaults_seed_mcp_block`. If a
        // captain updates defaults.toml without also updating the
        // `Default` impl (or vice versa), the two tests diverge and
        // the suite fails.
        let from_toml: super::super::Config = toml::from_str(super::super::DEFAULTS).expect("defaults parse");
        assert_eq!(
            from_toml.mcp,
            McpConfig::default(),
            "[mcp] defaults.toml seed must match McpConfig::default()",
        );
    }
}
