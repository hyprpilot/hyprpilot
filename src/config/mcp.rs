//! `[mcp]` config block — controls MCP defaults, the in-tree
//! `hyprpilot` MCP server the launcher auto-injects into the vendor's
//! MCP config, and owns the **skills catalog** that server exposes.
//!
//! Two distinct concerns share the TOML neighbourhood: `[mcp]` is the
//! config for our in-tree MCP server; a profile's `mcps` is the
//! captain-declared catalog of *external* MCP servers.
//!
//! Skills live under `[[mcp.skills]]`. They belong here because the
//! hyprpilot MCP server is what exposes them to the agent; there is no
//! consumer of skills outside the MCP server.
//!
//! Per-profile `ProfileConfig.mcp: Option<McpConfig>` is folded onto
//! whichever profile is picked (root `[[patches]]` seed the default
//! `mcp` block; a per-profile `mcp` wholesale-replaces it) — mirroring
//! the `mcps` profile-override pattern. Field shapes are `Option<T>`
//! so the `overwrite_some` merge strategy can layer defaults →
//! patches → per-profile cleanly.

use garde::Validate;
use merge::Merge;
use serde::{Deserialize, Serialize};

use super::extensions::validate_globs;
use super::merge_strategies::overwrite_some;
use super::SkillEntry;

/// Default MCP server name for the skills surface.
///
/// **Renaming changes tool attribution** — `mcp__hyprpilot-skills__read_skill`
/// becomes `mcp__<name>__read_skill` — so any skill or instruction file
/// that names a tool by its prefix breaks with it. The `hyprpilot://`
/// resource URIs are a fixed scheme and are NOT affected.
pub const DEFAULT_SKILLS_SERVER_NAME: &str = "hyprpilot-skills";

/// Default MCP server name for the harness surface.
pub const DEFAULT_HARNESS_SERVER_NAME: &str = "hyprpilot-harness";

/// `[mcp.skills]` — the auto-injected skills server.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
#[merge(strategy = overwrite_some)]
pub struct SkillsServerConfig {
    /// Defaults to `true` — the historical behaviour, where configuring
    /// any skill root was enough to get the server.
    #[garde(skip)]
    pub enabled: Option<bool>,

    /// Server name in the vendor's MCP catalog. Reserved: a
    /// user-declared server of the same name is replaced by this one.
    #[garde(skip)]
    pub name: Option<String>,

    /// Skill catalog roots. Each is a directory of `<slug>/SKILL.md`
    /// bundles plus an optional per-root glob `ignore` list applied only
    /// to that root's discoveries.
    #[garde(dive)]
    pub roots: Option<Vec<SkillEntry>>,

    /// Per-server tool policy. Falls back to the `[mcp]`-level globs.
    #[garde(custom(validate_globs))]
    pub auto_accept_tools: Option<Vec<String>>,
    #[garde(custom(validate_globs))]
    pub auto_reject_tools: Option<Vec<String>>,
}

/// `[mcp.harness]` — the auto-injected agent-harness server.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
#[merge(strategy = overwrite_some)]
pub struct HarnessServerConfig {
    /// Defaults to `false`. See [`McpConfig::harness`] for why.
    #[garde(skip)]
    pub enabled: Option<bool>,

    #[garde(skip)]
    pub name: Option<String>,

    /// Sessions retained before the oldest finished ones are evicted.
    #[garde(skip)]
    pub max_sessions: Option<usize>,

    /// Per-server tool policy. Falls back to the `[mcp]`-level globs —
    /// worth tightening here, since `spawn` is the tool that runs
    /// arbitrary binaries.
    #[garde(custom(validate_globs))]
    pub auto_accept_tools: Option<Vec<String>>,
    #[garde(custom(validate_globs))]
    pub auto_reject_tools: Option<Vec<String>>,
}

impl SkillsServerConfig {
    pub fn is_enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }

    pub fn server_name(&self) -> &str {
        self.name.as_deref().unwrap_or(DEFAULT_SKILLS_SERVER_NAME)
    }
}

impl HarnessServerConfig {
    pub fn is_enabled(&self) -> bool {
        self.enabled.unwrap_or(false)
    }

    pub fn server_name(&self) -> &str {
        self.name.as_deref().unwrap_or(DEFAULT_HARNESS_SERVER_NAME)
    }
}

/// `[mcp]` block. Controls auto-injection of the in-tree
/// `hyprpilot mcp serve` MCP server entry into the vendor's MCP
/// catalog, owns the **skills catalog** that server exposes, and
/// provides default tool glob policy for MCP servers that do not
/// declare their own `hyprpilot` extension.
///
/// `auto_accept_tools` / `auto_reject_tools` ride through to the
/// auto-injected entry's `HyprpilotExtension` namespace key and are
/// also copied onto user-declared MCP definitions with no per-server
/// override, so every server carries one uniform per-server glob
/// shape when the policy is projected onto the vendor's native
/// approval flags. The default `["*"]` accept makes MCP calls
/// frictionless; captains can tighten per-profile or per-server.
///
/// `skills` is the **catalog of skill root directories** the server
/// scans and exposes. Same `SkillEntry { dir, ignore }` shape that
/// used to live at the top-level `[[skills]]`. `None` (default) →
/// falls through to the seeded default
/// (`~/.config/hyprpilot/skills`). `Some([])` → no skills at all
/// (suppresses auto-inject — nothing to serve). `Some([...])` →
/// wholesale-replaces the default catalog.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
#[merge(strategy = overwrite_some)]
pub struct McpConfig {
    /// `true` (default — `McpConfig::default()`, backfilled onto every
    /// resolved profile) → the launcher auto-injects the in-tree MCP
    /// server when `skills` resolves to a non-empty catalog. `false` →
    /// skip auto-inject entirely; agent sees no `hyprpilot` server.
    /// Profile-level `false` override lets the captain run a
    /// vendor-only launch.
    #[garde(skip)]
    pub enabled: Option<bool>,

    /// The skills server — `hyprpilot mcp skills`, exposing the skill
    /// catalog. On by default.
    #[garde(dive)]
    pub skills: Option<SkillsServerConfig>,

    /// The agent harness — `hyprpilot mcp harness`, exposing
    /// `list_profiles` / `spawn` / `session_*`.
    ///
    /// **Off by default, and that is a security property, not a
    /// preference.** A profile's `command` is an arbitrary binary, so
    /// anything that can call `spawn` executes commands as this user.
    #[garde(dive)]
    pub harness: Option<HarnessServerConfig>,

    /// Default glob patterns matching MCP tool leaf names for
    /// auto-accept. Default `["*"]` (`McpConfig::default()`) → every
    /// MCP tool on servers without a stricter per-server extension is
    /// projected as auto-approved onto the vendor's native approval
    /// surface. Tighten by setting
    /// `auto_accept_tools = ["list_*", "read_*"]`. Each pattern must
    /// be a valid glob — a malformed one rejects at config-load.
    #[garde(custom(validate_globs))]
    pub auto_accept_tools: Option<Vec<String>>,

    /// Glob patterns for auto-reject. Default `[]`
    /// (`McpConfig::default()`) — no rejects, every tool falls through
    /// to the accept set. Reject beats accept on overlap. Each pattern
    /// must be a valid glob — a malformed one rejects at config-load.
    #[garde(custom(validate_globs))]
    pub auto_reject_tools: Option<Vec<String>>,
}

impl Default for McpConfig {
    /// The per-leaf **fallback** the resolver backfills onto every
    /// profile's (possibly partial or absent) `[mcp]` block via
    /// `resolve::effective_mcp_with`. Carries only the non-path value
    /// leaves the `.expect()` accessors below require —
    /// `enabled = true`, `autoAcceptTools = ["*"]`,
    /// `autoRejectTools = []`.
    ///
    /// **`skills` is deliberately `None` here.** The XDG skills-dir
    /// default is the single load-bearing value that must survive
    /// config-layer merge, so it lives ONLY in the default
    /// `[[patches]]` entry (a config layer, additive across layers) —
    /// duplicating it here would be the `defaults.toml`-drift
    /// anti-pattern `CLAUDE.md` warns against. A profile that ends up
    /// with no `mcp.skills` (the seed patch was cleared, or a
    /// programmatic `Config`) simply auto-injects nothing.
    fn default() -> Self {
        Self {
            enabled: Some(true),
            skills: None,
            harness: None,
            auto_accept_tools: Some(vec!["*".to_string()]),
            auto_reject_tools: Some(Vec::new()),
        }
    }
}

impl McpConfig {
    /// `enabled.expect(...)` — infallible in practice: the resolver
    /// backfills the block onto `McpConfig::default()` (which seeds
    /// `enabled`) before any consumer reads it, so this can only fire
    /// if a caller constructs a raw partial `McpConfig` by hand.
    #[must_use]
    pub fn enabled(&self) -> bool {
        self.enabled
            .expect("[mcp] enabled seeded by McpConfig::default() (backfilled in resolve::effective_mcp_with)")
    }

    /// Auto-accept globs as a borrowed slice. Defaults to `["*"]` per
    /// `McpConfig::default()`, backfilled by `effective_mcp_with`.
    #[must_use]
    pub fn auto_accept_tools(&self) -> &[String] {
        self.auto_accept_tools
            .as_deref()
            .expect("[mcp] autoAcceptTools seeded by McpConfig::default() (backfilled in resolve::effective_mcp_with)")
    }

    /// Auto-reject globs as a borrowed slice. Defaults to `[]` per
    /// `McpConfig::default()`, backfilled by `effective_mcp_with`.
    #[must_use]
    pub fn auto_reject_tools(&self) -> &[String] {
        self.auto_reject_tools
            .as_deref()
            .expect("[mcp] autoRejectTools seeded by McpConfig::default() (backfilled in resolve::effective_mcp_with)")
    }

    /// Resolve every `[[mcp.skills]]` entry to its absolute path +
    /// compiled ignore matcher. `~` / `$VAR` expansion happens here.
    /// Returns an empty vec when `skills` is unset; treat that as
    /// "no skills" semantically.
    #[must_use]
    pub fn resolved_skills(&self) -> Vec<super::ResolvedSkillEntry> {
        self.skills
            .as_ref()
            .and_then(|skills| skills.roots.as_deref())
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
    fn default_carries_value_leaves_but_not_skills() {
        // `McpConfig::default()` is the per-leaf fallback the resolver
        // backfills onto every profile. It must seed exactly the value
        // leaves the `.expect()` accessors read — and must NOT carry
        // the XDG skills dir (that lives solely in the default
        // `[[patches]]` entry, single-sourced, so no `defaults.toml`
        // drift). A captain who moves the skills default into this
        // impl reintroduces the duplication and fails here.
        let d = McpConfig::default();
        assert_eq!(d.enabled, Some(true));
        assert_eq!(d.auto_accept_tools.as_deref(), Some(["*".to_string()].as_slice()));
        assert_eq!(d.auto_reject_tools.as_deref(), Some([].as_slice()));
        assert_eq!(
            d.skills, None,
            "skills default is single-sourced in the seed patch, not here"
        );
    }

    #[test]
    fn seed_patch_is_the_single_source_of_the_skills_dir() {
        // Pairs with `config::tests::defaults_seed_mcp_via_root_patch`.
        // The XDG skills dir lives in the default `[[patches]]` entry,
        // never in `McpConfig::default()` — deserialize the seed's
        // `mcp` sub-object and assert it carries the skills root while
        // the typed default does not.
        let from_toml: super::super::Config = toml::from_str(super::super::DEFAULTS).expect("defaults parse");
        let patches = from_toml.patches.as_deref().expect("defaults seed [[patches]]");
        let mcp_value = patches
            .iter()
            .find_map(|p| p.as_object()?.get("mcp"))
            .expect("default patch carries an mcp field");
        let seed_mcp: McpConfig =
            serde_json::from_value(mcp_value.clone()).expect("patch's mcp deserializes as McpConfig");
        let skills = seed_mcp
            .skills
            .as_ref()
            .and_then(|s| s.roots.as_deref())
            .expect("seed patch carries the skills roots");
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].dir, std::path::PathBuf::from("~/.config/hyprpilot/skills"));
        assert!(McpConfig::default().skills.is_none());
    }

    #[test]
    fn valid_tool_policy_globs_validate() {
        let cfg = McpConfig {
            enabled: Some(true),
            auto_accept_tools: Some(vec!["read_*".into(), "list_*".into()]),
            auto_reject_tools: Some(vec!["delete_*".into()]),
            ..McpConfig::default()
        };
        cfg.validate().expect("well-formed tool-policy globs must validate");
    }

    #[test]
    fn malformed_auto_accept_glob_rejects_at_load() {
        // Point-4 (K-746): `[mcp].autoAcceptTools` / `autoRejectTools`
        // were previously `#[garde(skip)]`, so a malformed glob slipped
        // through to match time. They now validate like the
        // `mcps`/`skills` ignore globs.
        let cfg = McpConfig {
            enabled: Some(true),
            auto_accept_tools: Some(vec!["[unterminated".into()]),
            auto_reject_tools: None,
            ..McpConfig::default()
        };
        let err = cfg.validate().expect_err("malformed accept glob must reject");
        assert!(err.to_string().contains("not a valid glob"), "got: {err}");
    }

    #[test]
    fn malformed_auto_reject_glob_rejects_at_load() {
        let cfg = McpConfig {
            enabled: Some(true),
            auto_accept_tools: None,
            auto_reject_tools: Some(vec!["nuke[".into()]),
            ..McpConfig::default()
        };
        let err = cfg.validate().expect_err("malformed reject glob must reject");
        assert!(err.to_string().contains("not a valid glob"), "got: {err}");
    }
}
