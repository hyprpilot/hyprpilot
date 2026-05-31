//! `[mcp]` config block — controls MCP defaults, the in-tree
//! `hyprpilot` MCP server the daemon auto-injects into ACP bootstrap
//! `mcp_servers`, and owns the **skills catalog** the server exposes.
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
/// `hyprpilot mcp serve` MCP server entry into ACP bootstrap
/// `mcp_servers` arrays, owns the **skills catalog** that server
/// exposes, and provides default tool glob policy for
/// MCP servers that do not declare their own `hyprpilot` extension.
///
/// `auto_accept_tools` / `auto_reject_tools` ride through to the
/// auto-injected entry's `HyprpilotExtension` namespace key and are
/// also copied onto user-declared MCP definitions with no per-server
/// override. `PermissionController::decide` then sees one uniform
/// per-server glob shape. The default `["*"]` accept makes MCP calls
/// frictionless; captains can tighten per-profile or per-server.
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

    /// Default glob patterns matching MCP tool leaf names for
    /// auto-accept. Default `["*"]` (seeded by defaults.toml) → every
    /// MCP `tools/call` on servers without a stricter per-server
    /// extension short-circuits to `Decision::Allow` at
    /// `PermissionController::decide` lane 2. Tighten by setting
    /// `auto_accept_tools = ["list_*", "read_*"]`.
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
                ignore_patterns: e.ignore.as_deref().map(<[String]>::to_vec).unwrap_or_default(),
                ignore: e.compile_ignore(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_matches_defaults_toml_seeded_patch_values() {
        // Pairs with `config::tests::defaults_seed_mcp_via_root_patch`.
        // `[mcp]` is no longer a root field — it's seeded via a
        // `[[patches]]` entry. Deserialize that patch's `mcp` sub-
        // object back into McpConfig and check it equals the typed
        // `Default::default()`. If a captain updates defaults.toml's
        // seeded mcp shape without also updating the `Default` impl
        // (or vice versa), the two paths diverge here.
        let from_toml: super::super::Config = toml::from_str(super::super::DEFAULTS).expect("defaults parse");
        let patches = from_toml.patches.as_deref().expect("defaults seed [[patches]]");
        let mcp_value = patches
            .iter()
            .find_map(|p| p.as_object()?.get("mcp"))
            .expect("default patch carries an mcp field");
        let mcp_from_patch: McpConfig =
            serde_json::from_value(mcp_value.clone()).expect("patch's mcp deserializes as McpConfig");
        assert_eq!(
            mcp_from_patch,
            McpConfig::default(),
            "default `[[patches]]` mcp seed must match McpConfig::default()",
        );
    }
}
