//! `[mcp]` config block — controls MCP defaults, the in-tree
//! `hyprpilot` MCP server the launcher auto-injects into the vendor's
//! MCP config, and owns the **skills catalog** that server exposes.
//!
//! Two distinct concerns share the TOML neighbourhood: `[mcp]` is the
//! config for our in-tree MCP server; a profile's `mcps` is the
//! captain-declared catalog of *external* MCP servers.
//!
//! Skill roots live under `[[mcp.skills.dirs]]`. They belong here
//! because the skills server is what exposes them to the agent. The
//! launcher's `resolve` also builds a registry, but only to decide
//! whether that server is worth injecting — it is the one in-tree
//! server also gated on content.
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
use super::merge_strategies::{merge_nested, overwrite_some};
use super::SkillEntry;

/// Fallbacks for the `[mcp.harness]` ceilings, used ONLY when no
/// `[[patches]]` seeded them — a programmatic `Config` in a test, or a
/// captain who cleared the seed.
///
/// The values a real launch reads live in `defaults.toml`, the file the
/// captain edits. These exist because `[mcp.harness]` is nested, so
/// unlike `[mcp]`'s own leaves it is not backfilled from
/// `McpConfig::default()` and an accessor cannot `.expect()` a value.
/// `defaults_seed_the_harness_ceilings` pins each one to its seeded
/// counterpart, so the pair cannot drift.
pub const DEFAULT_MAX_SPAWN_DEPTH: usize = 1;

/// See [`DEFAULT_MAX_SPAWN_DEPTH`] for why this fallback exists.
pub const DEFAULT_MAX_SESSIONS: usize = 64;

/// See [`DEFAULT_MAX_SPAWN_DEPTH`] for why this fallback exists.
///
/// Zero is unlimited, and it is the default: how many agents are worth
/// running at once is a property of the captain's machine and their
/// work, not something this crate can pick for them. The ceiling stays
/// available for a shared or resource-tight host.
pub const DEFAULT_MAX_LIVE_SESSIONS: usize = 0;

/// Fallback name for the skills surface — see
/// [`DEFAULT_MAX_SPAWN_DEPTH`] for why a nested block needs one.
///
/// The name a real launch INJECTS comes from `[mcp.skills] name`,
/// seeded in `defaults.toml`: the catalog key is a config value, not a
/// Rust one, so a captain renames a server by writing the field the
/// injector already reads. This constant covers only a `Config`
/// carrying no patches, and `defaults_seed_the_server_names` pins the
/// pair equal.
///
/// **Renaming changes tool attribution** — `mcp__hyprpilot-skills__read_skill`
/// becomes `mcp__<name>__read_skill` — so any skill or instruction file
/// that names a tool by its prefix breaks with it. The `hyprpilot://`
/// resource URIs are a fixed scheme and are NOT affected.
pub const DEFAULT_SKILLS_SERVER_NAME: &str = "hyprpilot-skills";

/// Fallback name for the harness surface — see
/// [`DEFAULT_SKILLS_SERVER_NAME`].
pub const DEFAULT_HARNESS_SERVER_NAME: &str = "hyprpilot-harness";

/// Fallback name for the general-tools surface — see
/// [`DEFAULT_SKILLS_SERVER_NAME`]. Keeps the bare `hyprpilot` name:
/// this is the server that grows whatever doesn't belong to a
/// dedicated surface, so it is the one a captain reaches for by the
/// product's own name — and with nothing to suffix, the kebab the
/// other two carry has nothing to separate.
pub const DEFAULT_TOOLS_SERVER_NAME: &str = "hyprpilot";

/// `[mcp.serve]` — the auto-injected general-tools server.
///
/// Home for tools that are neither a skills read nor an agent launch:
/// `open` today, whatever earns a place later. Kept off the skills
/// server so a captain can run one without the other, and so the
/// skills server's tool policy stays a statement about *skills*.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
#[merge(strategy = overwrite_some)]
pub struct ToolsServerConfig {
    /// Defaults to `true` — these tools are side-effect-light and were
    /// previously always present on the skills server.
    #[garde(skip)]
    pub enabled: Option<bool>,

    /// Server name in the vendor's MCP catalog. Reserved: a
    /// user-declared server of the same name is replaced by this one.
    #[garde(skip)]
    pub name: Option<String>,

    /// Per-server tool policy. Falls back to the `[mcp]`-level globs.
    #[garde(custom(validate_globs))]
    #[serde(alias = "auto_accept_tools")]
    pub auto_accept_tools: Option<Vec<String>>,
    #[garde(custom(validate_globs))]
    #[serde(alias = "auto_reject_tools")]
    pub auto_reject_tools: Option<Vec<String>>,
}

impl ToolsServerConfig {
    pub fn is_enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }

    pub fn server_name(&self) -> &str {
        self.name.as_deref().unwrap_or(DEFAULT_TOOLS_SERVER_NAME)
    }
}

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

    /// Skill catalog directories. Each is a directory of `<slug>/SKILL.md`
    /// bundles plus an optional per-root glob `ignore` list applied only
    /// to that root's discoveries.
    #[garde(dive)]
    pub dirs: Option<Vec<SkillEntry>>,

    /// Per-server tool policy. Falls back to the `[mcp]`-level globs.
    #[garde(custom(validate_globs))]
    #[serde(alias = "auto_accept_tools")]
    pub auto_accept_tools: Option<Vec<String>>,
    #[garde(custom(validate_globs))]
    #[serde(alias = "auto_reject_tools")]
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

    /// Finished sessions retained before the oldest are evicted. `0`
    /// retains every one of them.
    ///
    /// Counts only finished sessions: a running one holds a live model
    /// connection rather than history, and evicting it is exactly what
    /// this must never do. Were running sessions counted, a busy sidecar
    /// would spend its whole retention budget on work still in flight
    /// and drop the transcripts the cap exists to keep.
    #[garde(skip)]
    #[serde(alias = "max_sessions")]
    pub max_sessions: Option<usize>,

    /// Sessions allowed to run at once before `spawn` is refused. `0`
    /// (the default) allows any number.
    ///
    /// Breadth, where `max_depth` is recursion: a profile's `command` is
    /// an arbitrary binary, so a ceiling here is what stops one sidecar
    /// exhausting the host. Off by default because the right number is a
    /// property of the machine — set it on a shared or resource-tight
    /// host, leave it where an agent fanning out wide is the point.
    #[garde(skip)]
    #[serde(alias = "max_live_sessions")]
    pub max_live_sessions: Option<usize>,

    /// Push a completion event into the lead agent's context when a
    /// turn finishes. Defaults to `true`.
    ///
    /// Safe to leave on: a client that has not registered the channel
    /// drops the notification silently, and unknown capabilities are
    /// ignored per the MCP spec. The knob exists for NOISE — a session
    /// is `exited` after every turn, so a ten-turn conversation emits
    /// ten events.
    #[garde(skip)]
    #[serde(alias = "notify_on_complete")]
    pub notify_on_complete: Option<bool>,

    /// Per-server tool policy. Falls back to the `[mcp]`-level globs —
    /// worth tightening here, since `spawn` is the tool that runs
    /// arbitrary binaries.
    #[garde(custom(validate_globs))]
    #[serde(alias = "auto_accept_tools")]
    pub auto_accept_tools: Option<Vec<String>>,
    #[garde(custom(validate_globs))]
    #[serde(alias = "auto_reject_tools")]
    pub auto_reject_tools: Option<Vec<String>>,

    /// Which profiles THIS launch's harness may drive, as globs over
    /// profile ids. `None` (default) applies no filter; `Some([])` is
    /// the explicit "delegate to nothing" — the server is still
    /// injected, it just has no candidates.
    ///
    /// Distinct from `[profiles.harness]`, which is the TARGET's own
    /// opt-in and is global. This is the LAUNCHER's scope, so a
    /// `$match`ed patch can give `personal/*` a harness that reaches
    /// only `personal/*`. The two AND: a glob here can never promote a
    /// profile the captain never nominated.
    ///
    /// `globset`, so `*` crosses `/` exactly as `$match.profile` does —
    /// `personal/*` matches `personal/kilic/glm-5.2`.
    #[garde(custom(validate_globs))]
    #[serde(alias = "include_profiles")]
    pub include_profiles: Option<Vec<String>>,

    /// Glob deny-list over profile ids. Beats `include_profiles` on
    /// overlap, mirroring `excludeTools` / `autoRejectTools`.
    #[garde(custom(validate_globs))]
    #[serde(alias = "exclude_profiles")]
    pub exclude_profiles: Option<Vec<String>>,

    /// How many levels of delegation this harness allows. Defaults to
    /// [`DEFAULT_MAX_SPAWN_DEPTH`].
    ///
    /// Read in exactly one place — the `[mcp.harness]` block a gate is
    /// deciding whether to act on — so it answers both questions at
    /// once: whether a session at depth `d` gets a harness INJECTED
    /// (`d < maxDepth`), and whether a running sidecar at depth `d` may
    /// `spawn` (same comparison). `0` denies both everywhere.
    ///
    /// Raising it is a resource decision, not a security one: a
    /// `maxLiveSessions` ceiling, where one is set at all, bounds one
    /// sidecar's own table, so N delegates each running N sessions is a
    /// fan-out no single ceiling catches and the lead cannot see.
    /// Deliberately unbounded — the captain owns that
    /// trade, and a validator guessing a ceiling would only be wrong
    /// somewhere else.
    #[garde(skip)]
    #[serde(alias = "max_depth")]
    pub max_depth: Option<usize>,

    /// The `[mcp]` block every delegate this harness spawns receives,
    /// overlaid per-leaf onto the delegate profile's own resolved
    /// `[mcp]`: a key set here wins, a key left unset inherits.
    ///
    /// The launching session's answer to "what should the agents I
    /// delegate to be able to reach?" — distinct from `includeProfiles`,
    /// which answers "which of them may I reach at all?".
    ///
    /// `Box` because the type is mutually recursive (`McpConfig` ->
    /// `HarnessServerConfig` -> `McpConfig`); an unboxed `Option` would
    /// not terminate the size computation. Nesting past the first level
    /// is inert whenever `maxDepth` denies the delegate a harness of its
    /// own, which is the default.
    #[garde(dive)]
    pub mcp: Option<Box<McpConfig>>,
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

    #[must_use]
    pub fn notifies_on_complete(&self) -> bool {
        self.notify_on_complete.unwrap_or(true)
    }

    /// Delegation levels this harness allows — see [`Self::max_depth`].
    #[must_use]
    pub fn max_depth(&self) -> usize {
        self.max_depth.unwrap_or(DEFAULT_MAX_SPAWN_DEPTH)
    }

    /// Finished sessions retained per sidecar — see
    /// [`Self::max_sessions`].
    #[must_use]
    pub fn max_sessions(&self) -> usize {
        self.max_sessions.unwrap_or(DEFAULT_MAX_SESSIONS)
    }

    /// Sessions this sidecar may run at once — see
    /// [`Self::max_live_sessions`].
    #[must_use]
    pub fn max_live_sessions(&self) -> usize {
        self.max_live_sessions.unwrap_or(DEFAULT_MAX_LIVE_SESSIONS)
    }

    /// The delegate overlay, unboxed. Absent declares nothing, which
    /// leaves every delegate on its own resolved `[mcp]`.
    #[must_use]
    pub fn delegate_mcp(&self) -> Option<&McpConfig> {
        self.mcp.as_deref()
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
/// scans and exposes, as `SkillEntry { dir, ignore }`. `None`
/// (default) →
/// falls through to the seeded default
/// (`~/.config/hyprpilot/skills`). `Some([])` → no skills at all
/// (suppresses auto-inject — nothing to serve). `Some([...])` →
/// wholesale-replaces the default catalog.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default = "McpConfig::sparse", deny_unknown_fields, rename_all = "camelCase")]
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

    /// The general-tools server — `hyprpilot mcp serve`, exposing
    /// `open`. On by default.
    ///
    /// `merge_nested`, not the struct-level `overwrite_some`: a partial
    /// override block must keep the sibling leaves the other layer set.
    #[garde(dive)]
    #[merge(strategy = merge_nested)]
    pub serve: Option<ToolsServerConfig>,

    /// The skills server — `hyprpilot mcp skills`, exposing the skill
    /// catalog. On by default.
    ///
    /// `merge_nested` for the reason above, and here it is load-bearing:
    /// wholesale replacement would let an overlay that sets only
    /// `enabled` take `dirs` with it.
    #[garde(dive)]
    #[merge(strategy = merge_nested)]
    pub skills: Option<SkillsServerConfig>,

    /// The agent harness — `hyprpilot mcp harness`, exposing
    /// `list_profiles` / `spawn` / `session_*`.
    ///
    /// **Off by default, and that is a security property, not a
    /// preference.** A profile's `command` is an arbitrary binary, so
    /// anything that can call `spawn` executes commands as this user.
    #[garde(dive)]
    #[merge(strategy = merge_nested)]
    pub harness: Option<HarnessServerConfig>,

    /// Default glob patterns matching MCP tool leaf names for
    /// auto-accept. Default `["*"]` (`McpConfig::default()`) → every
    /// MCP tool on servers without a stricter per-server extension is
    /// projected as auto-approved onto the vendor's native approval
    /// surface. Tighten by setting
    /// `auto_accept_tools = ["list_*", "read_*"]`. Each pattern must
    /// be a valid glob — a malformed one rejects at config-load.
    #[garde(custom(validate_globs))]
    #[serde(alias = "auto_accept_tools")]
    pub auto_accept_tools: Option<Vec<String>>,

    /// Glob patterns for auto-reject. Default `[]`
    /// (`McpConfig::default()`) — no rejects, every tool falls through
    /// to the accept set. Reject beats accept on overlap. Each pattern
    /// must be a valid glob — a malformed one rejects at config-load.
    #[garde(custom(validate_globs))]
    #[serde(alias = "auto_reject_tools")]
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
            serve: None,
            skills: None,
            harness: None,
            auto_accept_tools: Some(vec!["*".to_string()]),
            auto_reject_tools: Some(Vec::new()),
        }
    }
}

impl McpConfig {
    /// The **deserialization** default: every leaf absent.
    ///
    /// Distinct from [`Default`], which seeds the resolver's per-leaf
    /// floor (`enabled = true`, `autoAcceptTools = ["*"]`). Container
    /// `#[serde(default)]` fills missing fields from `Self::default()`,
    /// so a partial block on disk used to come back carrying values its
    /// author never wrote — invisible where `effective_mcp_with`
    /// backfills the same numbers anyway, and wrong for
    /// `[mcp.harness.mcp]`, where an unwritten leaf must INHERIT the
    /// delegate's own rather than override it with a default.
    fn sparse() -> Self {
        Self {
            enabled: None,
            serve: None,
            skills: None,
            harness: None,
            auto_accept_tools: None,
            auto_reject_tools: None,
        }
    }

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
            .and_then(|skills| skills.dirs.as_deref())
            .unwrap_or(&[])
            .iter()
            .map(|e| super::ResolvedSkillEntry {
                dir: crate::paths::resolve_user(&e.dir.to_string_lossy()),
                ignore_patterns: e.ignore.as_deref().map(<[String]>::to_vec).unwrap_or_default(),
                ignore: e.compile_ignore(),
                watch: e.watches(),
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
            .and_then(|s| s.dirs.as_deref())
            .expect("seed patch carries the skills dirs");
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].dir, std::path::PathBuf::from("~/.config/hyprpilot/skills"));
        assert!(McpConfig::default().skills.is_none());
    }

    /// The wipe `merge_nested` exists to prevent. An overlay that names
    /// only `skills.enabled` used to replace the whole `skills` block
    /// under `overwrite_some`, taking `dirs` with it — every delegate
    /// silently loses its skill catalogue.
    #[test]
    fn a_partial_nested_block_keeps_the_sibling_leaves() {
        let mut base = McpConfig {
            skills: Some(SkillsServerConfig {
                dirs: Some(vec![SkillEntry {
                    dir: std::path::PathBuf::from("/skills"),
                    ignore: None,
                    watch: None,
                }]),
                ..Default::default()
            }),
            ..McpConfig::default()
        };
        base.merge(McpConfig {
            skills: Some(SkillsServerConfig {
                enabled: Some(false),
                ..Default::default()
            }),
            ..Default::default()
        });

        let skills = base.skills.expect("the block survives");
        assert_eq!(skills.enabled, Some(false), "the overlay's leaf wins");
        assert_eq!(
            skills.dirs.as_deref().map(<[SkillEntry]>::len),
            Some(1),
            "a leaf the overlay never mentioned must survive"
        );
    }

    /// Direction, pinned on its own. `overwrite_some` is
    /// right-wins, so the delegate overlay has to be the RIGHT operand
    /// — reversed, an inherited `Some` would clobber it and the whole
    /// feature would no-op against exactly the config that motivated it.
    #[test]
    fn the_right_operand_wins_leaf_by_leaf() {
        let mut base = McpConfig {
            harness: Some(HarnessServerConfig {
                enabled: Some(true),
                max_sessions: Some(7),
                ..Default::default()
            }),
            ..McpConfig::default()
        };
        base.merge(McpConfig {
            harness: Some(HarnessServerConfig {
                enabled: Some(false),
                ..Default::default()
            }),
            ..Default::default()
        });

        let harness = base.harness.expect("the block survives");
        assert_eq!(harness.enabled, Some(false), "a set leaf on the right overrides");
        assert_eq!(harness.max_sessions, Some(7), "an unset leaf on the right inherits");
    }

    /// The `[mcp]` block `defaults.toml` seeds, as the resolver sees it.
    fn seeded_mcp() -> McpConfig {
        let cfg: super::super::Config = toml::from_str(super::super::DEFAULTS).expect("defaults parse");
        let patches = cfg.patches.as_deref().expect("defaults seed [[patches]]");
        let mcp_value = patches
            .iter()
            .find_map(|p| p.as_object()?.get("mcp"))
            .expect("the default patch carries an mcp field");
        serde_json::from_value(mcp_value.clone()).expect("the seed deserializes")
    }

    /// `defaults.toml` is where a captain edits these numbers; the Rust
    /// constants only cover a `Config` carrying no patches at all. Pin
    /// them equal so the pair cannot drift into two different answers.
    #[test]
    fn defaults_seed_the_harness_ceilings() {
        let seeded = seeded_mcp();
        let harness = seeded.harness.expect("the seed carries [mcp.harness]");

        assert_eq!(harness.max_depth, Some(DEFAULT_MAX_SPAWN_DEPTH));
        assert_eq!(harness.max_sessions, Some(DEFAULT_MAX_SESSIONS));
        assert_eq!(harness.max_live_sessions, Some(DEFAULT_MAX_LIVE_SESSIONS));
        assert_eq!(
            harness.notify_on_complete,
            Some(HarnessServerConfig::default().notifies_on_complete())
        );
        assert_eq!(
            harness.enabled, None,
            "seeding `enabled` would turn the harness on for everyone — it stays the captain's call"
        );
    }

    /// The name a server is INJECTED under is `[mcp.*] name`, and the
    /// injector reads nothing else — so the seed is the real default and
    /// the constants only cover a `Config` carrying no patches. Pinned
    /// equal for the same reason the ceilings are.
    #[test]
    fn defaults_seed_the_server_names() {
        let seeded = seeded_mcp();

        assert_eq!(
            seeded.serve.expect("the seed carries [mcp.serve]").name.as_deref(),
            Some(DEFAULT_TOOLS_SERVER_NAME)
        );
        assert_eq!(
            seeded.skills.expect("the seed carries [mcp.skills]").name.as_deref(),
            Some(DEFAULT_SKILLS_SERVER_NAME)
        );
        assert_eq!(
            seeded.harness.expect("the seed carries [mcp.harness]").name.as_deref(),
            Some(DEFAULT_HARNESS_SERVER_NAME)
        );
    }

    /// A partial block must come back carrying ONLY what its author
    /// wrote. Container `#[serde(default)]` would fill the rest from
    /// `Default`, which seeds `enabled = true` and `["*"]` — invisible
    /// where the resolver backfills the same values anyway, and wrong
    /// for a delegate overlay, where an unwritten leaf has to inherit
    /// the delegate's own instead of overriding it.
    #[test]
    fn a_partial_block_deserializes_sparse_not_seeded() {
        let parsed: McpConfig = serde_json::from_str(r#"{"serve":{"enabled":false}}"#).expect("parses");

        assert_eq!(parsed.serve.and_then(|s| s.enabled), Some(false));
        assert_eq!(parsed.enabled, None, "an unwritten leaf must stay unwritten");
        assert_eq!(parsed.auto_accept_tools, None, "including the ones Default seeds");
    }

    /// The alias has to be the OTHER spelling, which depends on whether
    /// the struct renames. `[mcp.*]` serialises camelCase so its alias is
    /// the snake one; a struct with no `rename_all` already serialises
    /// snake, so aliasing it to its own field name is a no-op that reads
    /// like coverage. Both directions pinned here, across both kinds of
    /// struct, so the mistake cannot come back silently.
    #[test]
    fn aliases_cover_the_other_spelling_on_both_kinds_of_struct() {
        // Renamed struct: wire is camelCase, alias adds snake.
        let renamed: HarnessServerConfig = toml::from_str("max_depth = 4\n").expect("snake alias on a camel struct");
        assert_eq!(renamed.max_depth, Some(4));

        // Unrenamed struct: wire is snake, alias must add camelCase.
        let plain: super::super::ProfileConfig =
            serde_json::from_value(serde_json::json!({ "id": "p", "agent": "a", "systemPrompt": [] }))
                .expect("camel alias on a snake struct");
        assert_eq!(plain.system_prompt.as_deref(), Some([].as_slice()));

        let plain: super::super::MultiplexerConfig =
            toml::from_str("setTitle = true\n").expect("camel alias on a snake struct");
        assert_eq!(plain.set_title, Some(true));
    }

    /// Both casings parse. `[mcp.*]` serialises camelCase, but TOML
    /// convention is snake_case and the rest of the config tree is
    /// snake, so a captain who writes either gets the same block.
    #[test]
    fn either_casing_parses_to_the_same_block() {
        let camel: HarnessServerConfig =
            toml::from_str("maxDepth = 2\nmaxSessions = 9\nmaxLiveSessions = 4\nnotifyOnComplete = false\n")
                .expect("camelCase parses");
        let snake: HarnessServerConfig =
            toml::from_str("max_depth = 2\nmax_sessions = 9\nmax_live_sessions = 4\nnotify_on_complete = false\n")
                .expect("snake_case parses");

        assert_eq!(camel, snake);
    }

    /// The cost of accepting both: they are distinct KEYS to the patch
    /// engine, which merges by string before anything is typed. Mixing
    /// spellings for one field therefore reaches serde as a duplicate
    /// and fails loudly — pinned so the failure is a known trade rather
    /// than a surprise, and documented at the seed in `defaults.toml`.
    #[test]
    fn mixing_casings_for_one_field_is_a_loud_error() {
        let err = toml::from_str::<HarnessServerConfig>("maxDepth = 2\nmax_depth = 3\n")
            .expect_err("one field, two spellings, one table");

        assert!(err.to_string().contains("duplicate"), "got: {err}");
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

    /// The delegate scope decides what an agent may execute, so a
    /// malformed pattern must not reach match time — where an
    /// uncompilable exclude would silently stop excluding.
    #[test]
    fn malformed_delegate_scope_globs_reject_at_load() {
        let cfg = McpConfig {
            harness: Some(HarnessServerConfig {
                enabled: Some(true),
                exclude_profiles: Some(vec!["work/[unterminated".into()]),
                ..Default::default()
            }),
            ..McpConfig::default()
        };
        let err = cfg.validate().expect_err("malformed exclude glob must reject");
        assert!(err.to_string().contains("not a valid glob"), "got: {err}");
    }

    #[test]
    fn well_formed_delegate_scope_globs_validate() {
        let cfg = McpConfig {
            harness: Some(HarnessServerConfig {
                enabled: Some(true),
                include_profiles: Some(vec!["personal/*".into()]),
                exclude_profiles: Some(vec!["personal/codex/*".into()]),
                ..Default::default()
            }),
            ..McpConfig::default()
        };
        cfg.validate().expect("well-formed delegate globs must validate");
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
