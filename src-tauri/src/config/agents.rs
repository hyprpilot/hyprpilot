//! `[[agents]]` + `[profile]` + `[[profiles]]`.
//! Cross-field reference checks (`profile.agent` → agents,
//! `[profile].default` → profiles) are wired into the garde walk at
//! the `Config` level via higher-order `custom(...)` hooks.

use std::collections::BTreeMap;
use std::path::PathBuf;

use garde::Validate;
use merge::Merge;
use serde::{Deserialize, Serialize};

use super::merge_strategies::{merge_agents_by_id, overwrite_some};
use super::validations::validate_agents_ids;

/// `[[agents]]` registry. Entries override by `id`; new ids append.
///
/// No `[agent] default` singleton anymore — every spawn flows through
/// a `[[profiles]]` entry (which carries its own `agent` field), so
/// the daemon never has to pick an agent independent of a profile.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields)]
pub struct AgentsConfig {
    #[garde(dive)]
    #[garde(custom(validate_agents_ids))]
    #[merge(strategy = merge_agents_by_id)]
    pub agents: Vec<AgentConfig>,
}

/// `[profile]` — global profile-scope singleton. `default` is the
/// `[[profiles]]` id used when `submit` doesn't carry an explicit
/// one. Spawn fails when neither `--profile` nor `[profile] default`
/// is set — there is no bare-agent fallback. Cross-field validation
/// against `[[profiles]].id` lives at `Config` level.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields)]
#[merge(strategy = overwrite_some)]
pub struct ProfileDefaults {
    #[garde(skip)]
    pub default: Option<String>,
}

/// One `[[agents]]` entry. No `permission_policy` — vendors own
/// that; client-side auto-accept/reject is a future
/// `PermissionController` issue (see CLAUDE.md).
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Validate)]
#[serde(deny_unknown_fields)]
pub struct AgentConfig {
    #[garde(length(min = 1))]
    pub id: String,
    #[garde(skip)]
    pub provider: AgentProvider,
    /// Vendor-translated at spawn time: env var or CLI flag per vendor.
    #[garde(skip)]
    pub model: Option<String>,
    /// Vendor-translated at spawn time when the adapter exposes a
    /// reasoning-effort/config override surface.
    #[garde(skip)]
    pub effort: Option<String>,
    /// Spawn binary. Mandatory — no per-provider fallback table at
    /// the trait layer. defaults.toml supplies one for every named
    /// provider; user `[[agents]]` entries (named or `acp`)
    /// must declare it explicitly.
    #[garde(length(min = 1))]
    pub command: String,
    #[garde(skip)]
    #[serde(default)]
    pub args: Vec<String>,
    /// Direct provider TUI command. Separate from `command` / `args`,
    /// which are the ACP bridge command for overlay-managed sessions.
    #[garde(dive)]
    pub spawn: Option<AgentSpawnConfig>,
    /// Missing → `std::env::current_dir()` at `new_session` time.
    #[garde(skip)]
    pub cwd: Option<PathBuf>,
    #[garde(skip)]
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Validate)]
#[serde(deny_unknown_fields)]
pub struct AgentSpawnConfig {
    #[garde(length(min = 1))]
    pub command: String,
    #[garde(skip)]
    #[serde(default)]
    pub args: Vec<String>,
}

/// Closed enum — each named variant maps to an `AcpAgent` impl with
/// hardcoded model + system-prompt injection behaviour. `Custom`
/// opens the door to user-supplied ACP binaries that need no
/// injection (or, in a follow-up, schema-driven injection from
/// `[[agents]]` TOML). Wire names are explicit to avoid `acp-open-code`
/// for `AcpOpenCode`.
///
/// `Acp*` prefix on every variant is deliberate — the protocol id is
/// part of the identity. A future `Http*` family lands as siblings, not
/// renames. Hence `clippy::enum_variant_names` allow.
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
pub enum AgentProvider {
    #[default]
    #[serde(rename = "acp-claude-code")]
    AcpClaudeCode,
    #[serde(rename = "acp-codex")]
    AcpCodex,
    #[serde(rename = "acp-opencode")]
    AcpOpenCode,
    /// User-supplied ACP-speaking binary. `command` / `args` are
    /// mandatory; injection knobs default to no-op (no model env or
    /// argv flag, no system-prompt injection). For vendors that need
    /// model env / system-prompt argv injection, copy one of the
    /// three named providers.
    #[serde(rename = "acp")]
    Acp,
}

impl AgentProvider {
    /// Wire id — the string serde produces / consumes for this variant.
    /// Single source of truth for the per-vendor identifier. Retained
    /// for callers that need the ascii vendor key without duplicating
    /// the literal; no live consumer after the formatter registry left.
    #[allow(dead_code)]
    pub const fn wire_id(self) -> &'static str {
        match self {
            Self::AcpClaudeCode => "acp-claude-code",
            Self::AcpCodex => "acp-codex",
            Self::AcpOpenCode => "acp-opencode",
            Self::Acp => "acp",
        }
    }
}

/// One `[[profiles]]` entry. Binds an agent id to an optional model
/// override + optional system prompt file. `system_prompt` is a path
/// only — there's exactly one mechanism. The file is read at resolve
/// time (not at spawn) so a missing file fails loudly on the next
/// submit, not silently at boot.
///
/// Per-server tool auto-accept / auto-reject lives inside each MCP
/// JSON entry's `hyprpilot` extension block (see `mcp/loader.rs`),
/// not on the profile. Profile-level customization happens via the
/// `mcps` field — pointing the profile at a different MCP file set
/// with stricter / looser per-server lists.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Validate)]
#[serde(deny_unknown_fields)]
pub struct ProfileConfig {
    #[garde(length(min = 1))]
    pub id: String,
    #[garde(length(min = 1))]
    pub agent: String,
    #[garde(inner(length(min = 1)))]
    pub model: Option<String>,
    /// Profile-level reasoning effort. Adapter-specific spawn code
    /// maps this common knob to the vendor's config surface.
    #[garde(inner(length(min = 1)))]
    pub effort: Option<String>,
    /// Profile-level system-prompt list. Same shape as the root
    /// `system_prompt`: array of `{ file, inject? }` entries.
    /// Captains compose layered prompts (base persona + project-
    /// specific addendum) by listing multiple entries; per-entry
    /// `inject` gates which bootstrap paths each file rides on.
    /// Profile-level value wholesale-replaces the root
    /// `system_prompt`; `system_prompt = []` is the explicit "no
    /// prompt" off-switch.
    #[garde(dive)]
    pub system_prompt: Option<Vec<crate::config::SystemPromptEntry>>,
    /// Profile-level MCP catalog. `None` (unset) → fall back to the
    /// global `[[mcps]]`. `Some(vec![…])` → wholesale replace the
    /// global default. `Some(vec![])` → no MCPs at all (explicit
    /// off-switch, no fallback). Same `McpFile { file, ignore }`
    /// shape as the root array.
    #[garde(dive)]
    pub mcps: Option<Vec<crate::config::McpFile>>,
    /// Profile-level `[mcp]` override. `None` (unset) → inherit the
    /// global `Config.mcp`. `Some(...)` → wholesale-replace the
    /// global block (mirrors `mcps` / `skills`); every field on the
    /// replacement uses its serde / defaults.toml default when
    /// omitted by the captain.
    #[garde(dive)]
    pub mcp: Option<crate::config::McpConfig>,
    /// Default mode id — free string today; validation against a mode
    /// catalog lands with the catalog.
    #[garde(inner(length(min = 1)))]
    pub mode: Option<String>,
    /// Profile-scoped cwd for the agent process. `~` expansion happens
    /// at consume time (mirrors `system_prompt`).
    #[garde(skip)]
    pub cwd: Option<PathBuf>,
    /// Extra env vars the agent process inherits. `BTreeMap` for
    /// deterministic serialisation; mirrors `AgentConfig.env`.
    #[serde(default)]
    #[garde(skip)]
    pub env: BTreeMap<String, String>,
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write;

    use super::super::{load, Config, DEFAULTS};
    use super::*;

    fn write_tmp(name: &str, body: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("hyprpilot-test-{}-{}", std::process::id(), name));
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(body.as_bytes()).unwrap();

        path
    }

    /// Mirrors `defaults_populate_every_daemon_window_field` for the
    /// agents registry. If the seeded entries drift — wrong provider
    /// name, missing id, policy variant removed — this fires before
    /// the daemon starts panicking at runtime against a bad schema.
    #[test]
    fn defaults_populate_every_required_agent_field() {
        let cfg: Config = toml::from_str(DEFAULTS).expect("defaults must parse");

        let ids: Vec<&str> = cfg.agents.agents.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["claude-code", "codex", "opencode"],
            "defaults must seed the three built-in vendors"
        );

        for a in &cfg.agents.agents {
            assert!(!a.command.is_empty(), "agents[{}].command", a.id);
            assert!(!a.args.is_empty(), "agents[{}].args", a.id);
            let spawn = a.spawn.as_ref().unwrap_or_else(|| panic!("agents[{}].spawn", a.id));
            assert!(!spawn.command.is_empty(), "agents[{}].spawn.command", a.id);
        }

        // Provider mapping per id.
        let by_id: std::collections::HashMap<&str, AgentProvider> =
            cfg.agents.agents.iter().map(|a| (a.id.as_str(), a.provider)).collect();
        assert_eq!(by_id["claude-code"], AgentProvider::AcpClaudeCode);
        assert_eq!(by_id["codex"], AgentProvider::AcpCodex);
        assert_eq!(by_id["opencode"], AgentProvider::AcpOpenCode);
    }

    #[test]
    fn user_agent_entry_overrides_default_by_id() {
        // Override claude-code's command; leave codex + opencode
        // untouched; add a new entry with a fresh id.
        let p = write_tmp(
            "agents.toml",
            r#"
[[agents]]
id = "claude-code"
provider = "acp-claude-code"
command = "my-claude"
args = ["--custom"]

[[agents]]
id = "my-local"
provider = "acp-codex"
command = "local-codex"
args = []
"#,
        );
        let cfg = load(Some(&p), None).expect("load");

        let ids: Vec<&str> = cfg.agents.agents.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["claude-code", "codex", "opencode", "my-local"],
            "overrides keep position, new ids append"
        );

        let cc = cfg.agents.agents.iter().find(|a| a.id == "claude-code").unwrap();
        assert_eq!(cc.command, "my-claude");
        assert_eq!(cc.args, vec!["--custom".to_string()]);

        // Untouched defaults keep everything.
        let codex = cfg.agents.agents.iter().find(|a| a.id == "codex").unwrap();
        assert_eq!(codex.command, "bunx");

        // Appended entry survived.
        let ml = cfg.agents.agents.iter().find(|a| a.id == "my-local").unwrap();
        assert_eq!(ml.provider, AgentProvider::AcpCodex);

        fs::remove_file(&p).ok();
    }

    #[test]
    fn validate_rejects_duplicate_agent_ids() {
        let p = write_tmp(
            "dup.toml",
            r#"
[[agents]]
id = "dupe"
provider = "acp-claude-code"
command = "a"

[[agents]]
id = "dupe"
provider = "acp-codex"
command = "b"
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        let err = cfg.validate().expect_err("should reject");
        let msg = err.to_string();
        assert!(msg.contains("duplicate agent id 'dupe'"), "{msg}");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn validate_rejects_empty_profiles_list() {
        // Bypass the defaults-merge path — exercise `validate()`
        // directly on a Config with `profiles = []`. Every spawn
        // flows through a profile so this must reject at
        // config-load instead of erroring per-spawn. (The actual
        // load() path always merges in the default seed profile so
        // captains never hit this in practice unless their TOML
        // explicitly `$patch: replace`s the profiles list.)
        let cfg = Config {
            profiles: Vec::new(),
            ..Default::default()
        };
        let err = cfg.validate().expect_err("empty profiles must reject");
        let msg = err.to_string();
        assert!(msg.contains("at least one [[profiles]] entry"), "missing detail: {msg}");
    }

    #[test]
    fn agent_provider_round_trips_kebab_case() {
        // Spot-check each variant. Names match the TOML literals in
        // defaults.toml — a rename would require updating defaults
        // AND every user config out there.
        for (v, literal) in [
            (AgentProvider::AcpClaudeCode, "\"acp-claude-code\""),
            (AgentProvider::AcpCodex, "\"acp-codex\""),
            (AgentProvider::AcpOpenCode, "\"acp-opencode\""),
        ] {
            assert_eq!(serde_json::to_string(&v).unwrap(), literal);
            let back: AgentProvider = serde_json::from_str(literal).unwrap();
            assert_eq!(back, v);
        }
    }

    #[test]
    fn agent_without_model_parses() {
        let p = write_tmp(
            "no-model.toml",
            r##"
[[agents]]
id = "bare"
provider = "acp-claude-code"
command = "my-agent"
args = ["--flag"]

[[profiles]]
id = "bare"
agent = "bare"
"##,
        );
        let cfg = load(Some(&p), None).expect("load");
        let bare = cfg.agents.agents.iter().find(|a| a.id == "bare").expect("bare entry");
        assert_eq!(bare.model, None, "model must be absent when not set in TOML");
        cfg.validate().expect("valid");
        fs::remove_file(&p).ok();
    }

    /// Defaults seed ZERO profiles + no `[profile] default`.
    /// Fresh installs configure at least one profile on disk;
    /// `validate_profiles_non_empty` rejects an empty list at
    /// config-load so the captain finds out at startup rather than
    /// per spawn. This shape avoids polluting the captain's profile
    /// list with a default-pretender profile that's not actually
    /// any of their setups.
    #[test]
    fn defaults_seed_no_profile() {
        let cfg: Config = toml::from_str(DEFAULTS).expect("defaults must parse");

        assert!(cfg.profiles.is_empty(), "defaults must NOT seed a profile");
        assert!(cfg.profile.default.is_none(), "[profile] default must NOT be seeded");

        // Validation rejects the empty defaults — captain must supply
        // a profile of their own.
        let err = cfg
            .validate()
            .expect_err("defaults must fail validation without a profile");
        assert!(
            err.to_string().contains("at least one [[profiles]] entry"),
            "got: {err}"
        );
    }

    #[test]
    fn profile_references_missing_agent_fails() {
        let p = write_tmp(
            "missing-agent.toml",
            r#"
[[profiles]]
id = "ghost"
agent = "does-not-exist"
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        let err = cfg.validate().expect_err("should reject");
        let msg = err.to_string();
        assert!(msg.contains("profile 'ghost'"), "{msg}");
        assert!(msg.contains("'does-not-exist'"), "{msg}");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn profile_ids_unique() {
        let p = write_tmp(
            "dup-profiles.toml",
            r#"
[[profiles]]
id = "dupe"
agent = "claude-code"

[[profiles]]
id = "dupe"
agent = "codex"
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        let err = cfg.validate().expect_err("should reject");
        let msg = err.to_string();
        assert!(msg.contains("duplicate profile id 'dupe'"), "{msg}");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn default_profile_references_missing_profile_fails() {
        let p = write_tmp(
            "bad-default-profile.toml",
            r#"
[profile]
default = "ghost-profile"
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        let err = cfg.validate().expect_err("should reject");
        let msg = err.to_string();
        assert!(msg.contains("default = 'ghost-profile'"), "{msg}");
        assert!(msg.contains("Configured ids:"), "{msg}");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn profile_parses_full_schema_without_mcp_files() {
        let p = write_tmp(
            "profile-full.toml",
            r#"
[[profiles]]
id = "full"
agent = "claude-code"
model = "claude-opus-4-5"
system_prompt = [
  { file = "~/.config/hyprpilot/prompts/base.md" },
  { file = "~/.config/hyprpilot/prompts/full.md", inject = { on_create = true, on_update = true } },
]
mode = "ask"
cwd = "~/work"

[profiles.mcp]
skills = [{ dir = "~/.claude/skills/rust" }, { dir = "~/.claude/skills/vue" }]

[profiles.env]
FOO = "bar"
BAZ = "qux"
"#,
        );
        let cfg = load(Some(&p), None).expect("load");
        let full = cfg.profiles.iter().find(|p| p.id == "full").expect("full entry");
        assert_eq!(full.model.as_deref(), Some("claude-opus-4-5"));
        let prompts = full.system_prompt.as_deref().expect("system_prompt set");
        assert_eq!(prompts.len(), 2);
        assert_eq!(
            prompts[0].file,
            std::path::PathBuf::from("~/.config/hyprpilot/prompts/base.md")
        );
        // Default inject: on_create=true, on_update=false.
        assert!(prompts[0].inject.on_create);
        assert!(!prompts[0].inject.on_update);
        assert_eq!(
            prompts[1].file,
            std::path::PathBuf::from("~/.config/hyprpilot/prompts/full.md")
        );
        assert!(prompts[1].inject.on_create);
        assert!(prompts[1].inject.on_update, "explicit on_update=true honoured");
        assert_eq!(full.mcps, None, "absent mcps parses as None");
        let mcp_block = full.mcp.as_ref().expect("[profiles.mcp] block parsed");
        let skills = mcp_block.skills.as_deref().expect("[profiles.mcp].skills set");
        assert_eq!(skills.len(), 2);
        assert_eq!(skills[0].dir, PathBuf::from("~/.claude/skills/rust"));
        assert_eq!(skills[1].dir, PathBuf::from("~/.claude/skills/vue"));
        assert_eq!(full.mode.as_deref(), Some("ask"));
        assert_eq!(full.cwd.as_deref(), Some(PathBuf::from("~/work")).as_deref());
        assert_eq!(full.env.get("FOO").map(String::as_str), Some("bar"));
        assert_eq!(full.env.get("BAZ").map(String::as_str), Some("qux"));
        cfg.validate().expect("valid full profile");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn profile_parses_mcps_array_of_tables() {
        let p = write_tmp(
            "profile-mcps-files.toml",
            r#"
[[profiles]]
id = "work"
agent = "claude-code"
mcps = [
  { file = "~/.config/hyprpilot/mcps/work.json" },
  { file = "/etc/hyprpilot/shared.json", ignore = ["scratch-*"] },
]
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        let work = cfg.profiles.iter().find(|p| p.id == "work").expect("work entry");
        let mcps = work.mcps.as_deref().expect("set");
        assert_eq!(mcps.len(), 2);
        assert_eq!(mcps[0].file, Some(PathBuf::from("~/.config/hyprpilot/mcps/work.json")));
        assert_eq!(mcps[1].file, Some(PathBuf::from("/etc/hyprpilot/shared.json")));
        assert_eq!(mcps[1].ignore.as_deref(), Some(&["scratch-*".to_string()][..]));
        cfg.validate().expect("valid mcps");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn profile_empty_mcps_means_no_mcps() {
        let p = write_tmp(
            "profile-empty-mcps.toml",
            r#"
[[profiles]]
id = "minimal"
agent = "claude-code"
mcps = []
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        let minimal = cfg.profiles.iter().find(|p| p.id == "minimal").expect("minimal");
        assert_eq!(
            minimal.mcps,
            Some(vec![]),
            "empty list parses as Some(vec![]) — explicit off-switch"
        );
        cfg.validate().expect("empty list validates");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn profile_rejects_invalid_mcps_ignore_glob() {
        let p = write_tmp(
            "bad-mcps-glob.toml",
            r#"
[[profiles]]
id = "busted"
agent = "claude-code"
mcps = [{ file = "/etc/hyprpilot/x.json", ignore = ["[unterminated"] }]
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        let err = cfg.validate().expect_err("should reject");
        assert!(err.to_string().contains("not a valid glob"), "{err}");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn profile_skills_parses_array_of_tables() {
        let p = write_tmp(
            "profile-skills.toml",
            r#"
[[profiles]]
id = "team"
agent = "claude-code"

[profiles.mcp]
skills = [
  { dir = "~/.claude/skills/rust" },
  { dir = "~/.claude/skills/all", ignore = ["work-*"] },
]
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        let team = cfg.profiles.iter().find(|p| p.id == "team").expect("team entry");
        let mcp_block = team.mcp.as_ref().expect("[profiles.mcp] parsed");
        let skills = mcp_block.skills.as_deref().expect("set");
        assert_eq!(skills.len(), 2);
        assert_eq!(skills[0].dir, PathBuf::from("~/.claude/skills/rust"));
        assert_eq!(skills[1].dir, PathBuf::from("~/.claude/skills/all"));
        assert_eq!(skills[1].ignore.as_deref(), Some(&["work-*".to_string()][..]));
        cfg.validate().expect("valid skills");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn profile_rejects_invalid_skills_ignore_glob() {
        let p = write_tmp(
            "bad-skills-glob.toml",
            r#"
[[profiles]]
id = "busted"
agent = "claude-code"

[profiles.mcp]
skills = [{ dir = "~/x", ignore = ["[unterminated"] }]
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        let err = cfg.validate().expect_err("should reject");
        assert!(err.to_string().contains("not a valid glob"), "{err}");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn profile_empty_skills_means_none() {
        let p = write_tmp(
            "empty-list-skills.toml",
            r#"
[[profiles]]
id = "deny"
agent = "claude-code"

[profiles.mcp]
skills = []
"#,
        );
        let cfg = load(Some(&p), None).expect("load");
        let deny = cfg.profiles.iter().find(|p| p.id == "deny").expect("deny entry");
        let mcp_block = deny.mcp.as_ref().expect("[profiles.mcp] block present");
        assert_eq!(mcp_block.skills, Some(vec![]));
        cfg.validate().expect("empty list validates");
        fs::remove_file(&p).ok();
    }

    #[test]
    /// User profiles ARE the full profile list — no defaults-seeded
    /// `claude-code` polluting the captain's view. `merge_profiles_by_id`
    /// folds onto the empty defaults vec so iter-order is exactly
    /// the captain's file order.
    fn user_profiles_flow_through_in_order() {
        let p = write_tmp(
            "user-profiles.toml",
            r#"
[[profiles]]
id = "strict"
agent = "opencode"
model = "custom-model"

[[profiles]]
id = "my-profile"
agent = "claude-code"
"#,
        );
        let cfg = load(Some(&p), None).expect("load");

        let ids: Vec<&str> = cfg.profiles.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["strict", "my-profile"],
            "user profiles ARE the full list; nothing seeded into the captain's view",
        );

        let strict = cfg.profiles.iter().find(|p| p.id == "strict").unwrap();
        assert_eq!(strict.agent, "opencode");
        assert_eq!(strict.model.as_deref(), Some("custom-model"));
        assert!(strict.system_prompt.is_none());

        fs::remove_file(&p).ok();
    }
}
