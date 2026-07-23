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
/// No `[agent] default` singleton — every launch flows through a
/// `[[profiles]]` entry (which carries its own `agent` field), so an
/// agent is never picked independent of a profile.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields)]
pub struct AgentsConfig {
    #[garde(dive)]
    #[garde(custom(validate_agents_ids))]
    #[merge(strategy = merge_agents_by_id)]
    pub agents: Vec<AgentConfig>,
}

/// `[profile]` — global profile-scope singleton. `default` is the
/// `[[profiles]]` id launched when `--profile`/`-p` isn't passed. The
/// launch fails when neither `--profile` nor `[profile] default`
/// resolves — there is no bare-agent fallback. Cross-field validation
/// against `[[profiles]].id` lives at `Config` level.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields)]
#[merge(strategy = overwrite_some)]
pub struct ProfileDefaults {
    #[garde(skip)]
    pub default: Option<String>,
}

/// One `[[agents]]` entry. No `permission_policy` on the agent —
/// vendors own approval; per-server MCP tool auto-accept/reject lives
/// inside each MCP JSON entry's `hyprpilot` extension block.
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
    /// Vendor-translated at spawn time when the provider exposes a
    /// reasoning-effort/config override surface.
    #[garde(skip)]
    pub effort: Option<String>,
    /// Native CLI binary the launcher `exec`s into. Mandatory — no
    /// per-provider fallback table. defaults.toml supplies one for
    /// every built-in provider; a user-authored `[[agents]]` entry
    /// must declare it explicitly.
    #[garde(length(min = 1))]
    pub command: String,
    #[garde(skip)]
    #[serde(default)]
    pub args: Vec<String>,
    /// Missing → `std::env::current_dir()` at spawn time.
    #[garde(skip)]
    pub cwd: Option<PathBuf>,
    #[garde(skip)]
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

/// Closed enum — each variant maps to a per-vendor native-CLI command
/// builder (model / effort / mode / system-prompt / MCP projection).
/// There is no generic/custom escape hatch: every agent must be one
/// of these predefined vendors, so every profile gets the full
/// projection (a hand-rolled CLI that wants none of this can still
/// declare its own `command` / `args` under one of these providers,
/// accepting that vendor's flag conventions). Wire names are explicit
/// so the vendor id stays stable.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
pub enum AgentProvider {
    #[default]
    #[serde(rename = "claude-code")]
    ClaudeCode,
    #[serde(rename = "codex")]
    Codex,
    #[serde(rename = "opencode")]
    OpenCode,
}

impl AgentProvider {
    /// Wire id — the string serde produces / consumes for this variant.
    /// Single source of truth for the per-vendor identifier; used by
    /// `spawn::launch_profile` to trace the resolved provider without
    /// duplicating the literal.
    pub const fn wire_id(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude-code",
            Self::Codex => "codex",
            Self::OpenCode => "opencode",
        }
    }
}

/// One `[[profiles]]` entry. Binds an agent id to an optional model
/// override + optional system prompt file. `system_prompt` is a path
/// only — there's exactly one mechanism. The file is read at resolve
/// time (not ahead of time) so a missing file fails loudly on the
/// next launch, not silently.
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
    /// Profile-level system-prompt list: array of `{ file, inject? }`
    /// entries. Captains compose layered prompts (base persona +
    /// project-specific addendum) by listing multiple entries;
    /// per-entry `inject` (default `true`) opts a file out of
    /// injection without removing it from the list. Shared prompts
    /// across profiles come from a root `[[patches]]` entry, which
    /// folds onto this list; `system_prompt = []` is the explicit
    /// "no prompt" off-switch.
    #[garde(dive)]
    pub system_prompt: Option<Vec<crate::config::SystemPromptEntry>>,
    /// Profile-level MCP catalog. `None` (unset) → whatever a root
    /// `[[patches]]` entry folds on (or empty). `Some(vec![…])` →
    /// wholesale-replace with this catalog. `Some(vec![])` → no MCPs
    /// at all (explicit off-switch). `McpFile { file, ignore }` or
    /// inline `mcp_servers` shape.
    #[garde(dive)]
    pub mcps: Option<Vec<crate::config::McpFile>>,
    /// Profile-level `[mcp]` override. `None` (unset) → whatever a
    /// root `[[patches]]` entry folds on (the defaults seed one).
    /// `Some(...)` → wholesale-replace that block (mirrors `mcps` /
    /// `skills`); every field on the replacement uses its serde /
    /// defaults.toml default when omitted by the captain.
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
    /// When `Some`, REPLACES the base `[[agents]]` entry's `command`
    /// wholesale for this profile only.
    #[garde(inner(length(min = 1)))]
    pub command: Option<String>,
    /// When `Some`, REPLACES the base agent's `args` wholesale — flags
    /// have no stable key to append/merge by (`--flag value`, `-c
    /// k=v`, positionals), so this is a wholesale swap, not an
    /// append. Captains who want to add one flag to an otherwise
    /// long agent-args list restate the full list here.
    #[garde(inner(inner(length(min = 1))))]
    pub args: Option<Vec<String>>,
    /// OVERLAYS onto the base agent's `env` per-key (profile key wins
    /// on collision). Absent keys leave the corresponding `agent.env`
    /// entry untouched.
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

    /// Pin the seeded `[[agents]]` registry shape. If the seeded
    /// entries drift — wrong provider name, missing id, non-native
    /// command — this fires before a spawn `exec`s the wrong binary.
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
        }

        // Provider mapping per id.
        let by_id: std::collections::HashMap<&str, AgentProvider> =
            cfg.agents.agents.iter().map(|a| (a.id.as_str(), a.provider)).collect();
        assert_eq!(by_id["claude-code"], AgentProvider::ClaudeCode);
        assert_eq!(by_id["codex"], AgentProvider::Codex);
        assert_eq!(by_id["opencode"], AgentProvider::OpenCode);

        // Defaults now `exec` the vendors' NATIVE CLIs directly — not
        // the old `bunx …-acp` bridge invocations.
        let by_cmd: std::collections::HashMap<&str, &str> = cfg
            .agents
            .agents
            .iter()
            .map(|a| (a.id.as_str(), a.command.as_str()))
            .collect();
        assert_eq!(by_cmd["claude-code"], "claude");
        assert_eq!(by_cmd["codex"], "codex");
        assert_eq!(by_cmd["opencode"], "opencode");
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
provider = "claude-code"
command = "my-claude"
args = ["--custom"]

[[agents]]
id = "my-local"
provider = "codex"
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

        // Untouched defaults keep the native CLI command.
        let codex = cfg.agents.agents.iter().find(|a| a.id == "codex").unwrap();
        assert_eq!(codex.command, "codex");

        // Appended entry survived.
        let ml = cfg.agents.agents.iter().find(|a| a.id == "my-local").unwrap();
        assert_eq!(ml.provider, AgentProvider::Codex);

        fs::remove_file(&p).ok();
    }

    #[test]
    fn validate_rejects_duplicate_agent_ids() {
        let p = write_tmp(
            "dup.toml",
            r#"
[[agents]]
id = "dupe"
provider = "claude-code"
command = "a"

[[agents]]
id = "dupe"
provider = "codex"
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
            (AgentProvider::ClaudeCode, "\"claude-code\""),
            (AgentProvider::Codex, "\"codex\""),
            (AgentProvider::OpenCode, "\"opencode\""),
        ] {
            assert_eq!(serde_json::to_string(&v).unwrap(), literal);
            let back: AgentProvider = serde_json::from_str(literal).unwrap();
            assert_eq!(back, v);
        }
    }

    /// K-741: `custom` (and any other unrecognised provider string)
    /// must reject at TOML parse time — `AgentProvider` is a closed
    /// set of the three predefined vendors, no generic escape hatch.
    #[test]
    fn unknown_provider_rejects_at_load() {
        let p = write_tmp(
            "custom-provider.toml",
            r#"
[[agents]]
id = "legacy"
provider = "custom"
command = "some-cli"
"#,
        );
        let err = load(Some(&p), None).expect_err("unknown provider must fail to load");
        let msg = err.to_string();
        assert!(msg.contains("custom") || msg.contains("provider"), "{msg}");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn agent_without_model_parses() {
        let p = write_tmp(
            "no-model.toml",
            r##"
[[agents]]
id = "bare"
provider = "claude-code"
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
  { file = "~/.config/hyprpilot/prompts/full.md", inject = false },
]
mode = "ask"
cwd = "~/work"
command = "claude-beta"
args = ["--fallback-model", "x"]

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
        // Default inject: true.
        assert!(prompts[0].inject);
        assert_eq!(
            prompts[1].file,
            std::path::PathBuf::from("~/.config/hyprpilot/prompts/full.md")
        );
        assert!(!prompts[1].inject, "explicit inject=false honoured");
        assert_eq!(full.mcps, None, "absent mcps parses as None");
        let mcp_block = full.mcp.as_ref().expect("[profiles.mcp] block parsed");
        let skills = mcp_block.skills.as_deref().expect("[profiles.mcp].skills set");
        assert_eq!(skills.len(), 2);
        assert_eq!(skills[0].dir, PathBuf::from("~/.claude/skills/rust"));
        assert_eq!(skills[1].dir, PathBuf::from("~/.claude/skills/vue"));
        assert_eq!(full.mode.as_deref(), Some("ask"));
        assert_eq!(full.cwd.as_deref(), Some(PathBuf::from("~/work")).as_deref());
        assert_eq!(full.command.as_deref(), Some("claude-beta"));
        assert_eq!(
            full.args.as_deref(),
            Some(&["--fallback-model".to_string(), "x".to_string()][..])
        );
        assert_eq!(full.env.get("FOO").map(String::as_str), Some("bar"));
        assert_eq!(full.env.get("BAZ").map(String::as_str), Some("qux"));
        cfg.validate().expect("valid full profile");
        fs::remove_file(&p).ok();
    }

    /// Absent `command` / `args` / `env` all parse to their no-op
    /// defaults (`None` / `None` / empty map) — the shape
    /// `from_profile_explicit` treats as "leave the base agent
    /// untouched".
    #[test]
    fn profile_flat_overrides_absent_parse_as_defaults() {
        let p = write_tmp(
            "profile-no-override.toml",
            r#"
[[profiles]]
id = "bare"
agent = "claude-code"
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        let bare = cfg.profiles.iter().find(|p| p.id == "bare").expect("bare entry");
        assert!(bare.command.is_none(), "command must default to None when absent");
        assert!(bare.args.is_none(), "args must default to None when absent");
        assert!(bare.env.is_empty(), "env must default to empty when absent");
        cfg.validate().expect("valid without flat overrides");
        fs::remove_file(&p).ok();
    }

    /// garde's `inner(inner(length(min = 1)))` rejects an
    /// empty-string element inside `args` — catches a captain's stray
    /// `args = [""]` at config-load instead of shipping a blank argv
    /// element to the vendor `exec`.
    #[test]
    fn profile_rejects_empty_string_arg() {
        let p = write_tmp(
            "profile-blank-override-arg.toml",
            r#"
[[profiles]]
id = "busted"
agent = "claude-code"
args = ["--flag", ""]
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        let err = cfg.validate().expect_err("empty-string arg must reject");
        assert!(err.to_string().contains("args"), "{err}");
    }

    /// garde's `inner(length(min = 1))` rejects an empty `command`
    /// string on the profile.
    #[test]
    fn profile_rejects_empty_command() {
        let p = write_tmp(
            "profile-blank-override-command.toml",
            r#"
[[profiles]]
id = "busted"
agent = "claude-code"
command = ""
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        let err = cfg.validate().expect_err("empty command must reject");
        assert!(err.to_string().contains("command"), "{err}");
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
