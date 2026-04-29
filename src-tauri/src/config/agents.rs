//! `[agent]` + `[[agents]]` + `[[profiles]]` + `[[mcps]]`. Cross-field
//! reference checks (`profile.agent` → agents, `profile.mcps` → mcps,
//! `agent.default_profile` → profiles) are wired into the garde walk
//! at the `Config` level via higher-order `custom(...)` hooks.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::Context;
use garde::Validate;
use globset::{Glob, GlobSet, GlobSetBuilder};
use merge::Merge;
use serde::{Deserialize, Serialize};

use super::merge_strategies::{merge_agents_by_id, overwrite_some};
use super::validations::{
    validate_agent_default_id, validate_agents_ids, validate_profile_tool_globs, validate_unique_nonempty,
};

/// `[[agents]]` registry + `[agent]` global scope. Entries override
/// by `id`; new ids append. Cross-field check on `agent.default`
/// closes over `&self.agents` via the garde custom hook.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields)]
pub struct AgentsConfig {
    #[garde(dive)]
    #[garde(custom(validate_agents_ids))]
    #[merge(strategy = merge_agents_by_id)]
    pub agents: Vec<AgentConfig>,
    #[garde(dive)]
    #[garde(custom(validate_agent_default_id(&self.agents)))]
    pub agent: AgentDefaults,
}

/// `[agent]` — global agent-scope config. Future timeout / cwd /
/// env knobs slot in here.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields)]
#[merge(strategy = overwrite_some)]
pub struct AgentDefaults {
    #[garde(skip)]
    pub default: Option<String>,
    #[garde(skip)]
    pub default_profile: Option<String>,
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
    /// Missing → vendor's default command.
    #[garde(inner(length(min = 1)))]
    pub command: Option<String>,
    #[garde(skip)]
    #[serde(default)]
    pub args: Vec<String>,
    /// Missing → `std::env::current_dir()` at `new_session` time.
    #[garde(skip)]
    pub cwd: Option<PathBuf>,
    #[garde(skip)]
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

/// Closed enum — each variant maps to an `AcpAgent` impl. Wire
/// names are explicit to avoid `acp-open-code` for `AcpOpenCode`.
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
}

/// One `[[profiles]]` entry. Binds an agent id to an optional model
/// override + optional system prompt. Exactly one of `system_prompt`
/// / `system_prompt_file` may be set; the file path is read at
/// resolve time, not at spawn time, so a missing file fails loudly.
///
/// `auto_accept_tools` / `auto_reject_tools` are glob patterns matched
/// against the ACP `ToolKind` wire name (falling back to the tool's
/// title when kind is absent) at permission-request time. Reject
/// beats accept; misses fall through to a user prompt. Patterns are
/// validated at load time — empty strings and invalid globs reject
/// with the profile id + offending pattern in the error.
///
/// Allowlists only apply to sessions that resolve through a profile.
/// Bare-agent sessions (no profile id on submit) always prompt the
/// user — the fallback to `[agent] default_profile` happens at
/// `ResolvedInstance` time in `adapters::profile`, so setting a
/// `default_profile` ensures allowlists are honored on unlabelled
/// submits.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Validate)]
#[serde(deny_unknown_fields)]
pub struct ProfileConfig {
    #[garde(length(min = 1))]
    pub id: String,
    #[garde(length(min = 1))]
    pub agent: String,
    #[garde(inner(length(min = 1)))]
    pub model: Option<String>,
    #[garde(inner(length(min = 1)))]
    pub system_prompt: Option<String>,
    #[garde(skip)]
    pub system_prompt_file: Option<PathBuf>,
    #[serde(default)]
    #[garde(custom(validate_profile_tool_globs(&self.id)))]
    pub auto_accept_tools: Vec<String>,
    #[serde(default)]
    #[garde(custom(validate_profile_tool_globs(&self.id)))]
    pub auto_reject_tools: Vec<String>,
    /// Opaque names from a future `[[mcps]]` catalog (K-270 introduces
    /// the catalog; reference-validation lands with it). `None` → all
    /// available; `Some(["all"])` → literal sentinel meaning all;
    /// `Some([])` → none; otherwise → the listed names verbatim.
    #[garde(custom(validate_unique_nonempty))]
    pub mcps: Option<Vec<String>>,
    /// Directory paths the skill loader scans (K-268). Follows the
    /// claude-code skill mechanism — each entry is a folder of
    /// manually-authored skills; the loader pulls them in at instance
    /// spawn. `None` → use defaults; `Some([])` → no skills.
    /// `~` expansion happens at consume time (mirrors `cwd` /
    /// `system_prompt_file`).
    #[garde(custom(validate_unique_nonempty))]
    pub skills: Option<Vec<PathBuf>>,
    /// Default mode id — free string today; validation against a mode
    /// catalog lands with the catalog.
    #[garde(inner(length(min = 1)))]
    pub mode: Option<String>,
    /// Profile-scoped cwd for the agent process. `~` expansion happens
    /// at consume time (mirrors `system_prompt_file`).
    #[garde(skip)]
    pub cwd: Option<PathBuf>,
    /// Extra env vars the agent process inherits. `BTreeMap` for
    /// deterministic serialisation; mirrors `AgentConfig.env`.
    #[serde(default)]
    #[garde(skip)]
    pub env: BTreeMap<String, String>,
}

impl ProfileConfig {
    /// Compile the accept/reject glob sets. Call once per resolved
    /// instance; `GlobSet` is immutable after build. Patterns are
    /// validated at TOML load time, so `unwrap()` on the build steps
    /// would also be fine — we return `Result` for robustness against
    /// hand-constructed `ProfileConfig` values in tests.
    pub fn compile_tool_globs(&self) -> anyhow::Result<(GlobSet, GlobSet)> {
        Ok((
            build_globset(&self.auto_accept_tools)?,
            build_globset(&self.auto_reject_tools)?,
        ))
    }
}

fn build_globset(patterns: &[String]) -> anyhow::Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for p in patterns {
        builder.add(Glob::new(p).with_context(|| format!("invalid glob pattern '{p}'"))?);
    }
    builder.build().context("failed to compile glob set")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write;

    use merge::Merge as _;

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

        assert_eq!(
            cfg.agents.agent.default.as_deref(),
            Some("claude-code"),
            "agent.default must be seeded to a concrete id"
        );

        let ids: Vec<&str> = cfg.agents.agents.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["claude-code", "codex", "opencode"],
            "defaults must seed the three built-in vendors"
        );

        for a in &cfg.agents.agents {
            assert!(a.command.is_some(), "agents[{}].command", a.id);
            assert!(!a.args.is_empty(), "agents[{}].args", a.id);
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
        assert_eq!(cc.command.as_deref(), Some("my-claude"));
        assert_eq!(cc.args, vec!["--custom".to_string()]);

        // Untouched defaults keep everything.
        let codex = cfg.agents.agents.iter().find(|a| a.id == "codex").unwrap();
        assert_eq!(codex.command.as_deref(), Some("bunx"));

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
    fn validate_rejects_unknown_agent_default() {
        let p = write_tmp(
            "bad-default.toml",
            r#"
[agent]
default = "does-not-exist"
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        let err = cfg.validate().expect_err("should reject");
        let msg = err.to_string();
        // garde's report prefixes the field path: the cross-field
        // custom is attached to `AgentsConfig.agent`, which flattens
        // to `agents.agent` on `Config`.
        assert!(msg.contains("agents.agent"), "missing path: {msg}");
        assert!(msg.contains("default = 'does-not-exist'"), "missing detail: {msg}");
        assert!(msg.contains("Configured ids:"), "missing id list: {msg}");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn user_override_of_agent_default_wins() {
        let p = write_tmp(
            "agent-default.toml",
            r#"
[agent]
default = "codex"
"#,
        );
        let cfg = load(Some(&p), None).expect("load");
        assert_eq!(cfg.agents.agent.default.as_deref(), Some("codex"));
        cfg.validate().expect("codex exists in defaults, so valid");
        fs::remove_file(&p).ok();
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
"##,
        );
        let cfg = load(Some(&p), None).expect("load");
        let bare = cfg.agents.agents.iter().find(|a| a.id == "bare").expect("bare entry");
        assert_eq!(bare.model, None, "model must be absent when not set in TOML");
        cfg.validate().expect("valid");
        fs::remove_file(&p).ok();
    }

    /// Defaults ship zero profiles and no `agent.default_profile` —
    /// profiles are user-supplied, the daemon falls back to the
    /// `[agent] default` agent when none is selected.
    #[test]
    fn defaults_seed_no_profiles() {
        let cfg: Config = toml::from_str(DEFAULTS).expect("defaults must parse");

        assert!(cfg.profiles.is_empty(), "defaults must not seed any profiles");
        assert!(
            cfg.agents.agent.default_profile.is_none(),
            "agent.default_profile must not be seeded"
        );

        cfg.validate().expect("empty profile set validates");
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
    fn profile_rejects_both_system_prompt_fields() {
        let p = write_tmp(
            "both-prompts.toml",
            r#"
[[profiles]]
id = "clash"
agent = "claude-code"
system_prompt = "inline"
system_prompt_file = "/tmp/whatever.md"
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        let err = cfg.validate().expect_err("should reject");
        let msg = err.to_string();
        assert!(
            msg.contains("system_prompt") && msg.contains("system_prompt_file"),
            "{msg}"
        );
        assert!(msg.contains("pick one"), "{msg}");
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
[agent]
default_profile = "ghost-profile"
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        let err = cfg.validate().expect_err("should reject");
        let msg = err.to_string();
        assert!(msg.contains("default_profile = 'ghost-profile'"), "{msg}");
        assert!(msg.contains("Configured ids:"), "{msg}");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn profile_without_allowlists_defaults_to_empty_vecs() {
        let p = write_tmp(
            "allowlists-default.toml",
            r#"
[[profiles]]
id = "plain"
agent = "claude-code"
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        let plain = cfg.profiles.iter().find(|p| p.id == "plain").expect("plain entry");
        assert!(plain.auto_accept_tools.is_empty());
        assert!(plain.auto_reject_tools.is_empty());
        cfg.validate().expect("empty allowlists validate");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn profile_parses_glob_allowlists() {
        let p = write_tmp(
            "allowlists.toml",
            r#"
[[profiles]]
id = "lax"
agent = "claude-code"
auto_accept_tools = ["Read", "Edit*"]
auto_reject_tools = ["Bash"]
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        let lax = cfg.profiles.iter().find(|p| p.id == "lax").expect("lax entry");
        assert_eq!(lax.auto_accept_tools, vec!["Read".to_string(), "Edit*".to_string()]);
        assert_eq!(lax.auto_reject_tools, vec!["Bash".to_string()]);
        cfg.validate().expect("valid glob set");
        let (accept, reject) = lax.compile_tool_globs().expect("compiles");
        assert!(accept.is_match("Read"));
        assert!(accept.is_match("EditFile"));
        assert!(!accept.is_match("Bash"));
        assert!(reject.is_match("Bash"));
        fs::remove_file(&p).ok();
    }

    #[test]
    fn profile_rejects_empty_glob_pattern() {
        let p = write_tmp(
            "empty-glob.toml",
            r#"
[[profiles]]
id = "bad"
agent = "claude-code"
auto_accept_tools = [""]
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        let err = cfg.validate().expect_err("should reject");
        let msg = err.to_string();
        assert!(msg.contains("profile 'bad'"), "{msg}");
        assert!(msg.contains("empty string"), "{msg}");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn profile_rejects_invalid_glob_pattern() {
        let p = write_tmp(
            "bad-glob.toml",
            r#"
[[profiles]]
id = "busted"
agent = "claude-code"
auto_reject_tools = ["["]
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        let err = cfg.validate().expect_err("should reject");
        let msg = err.to_string();
        assert!(msg.contains("profile 'busted'"), "{msg}");
        assert!(msg.contains("invalid tool glob"), "{msg}");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn profile_parses_full_schema() {
        let p = write_tmp(
            "profile-full.toml",
            r#"
[[mcps]]
name = "fs"
command = "uvx"
args = ["mcp-server-filesystem"]

[[mcps]]
name = "ripgrep"
command = "uvx"
args = ["mcp-server-ripgrep"]

[[profiles]]
id = "full"
agent = "claude-code"
model = "claude-opus-4-5"
system_prompt = "be terse"
mcps = ["fs", "ripgrep"]
skills = ["~/.claude/skills/rust", "~/.claude/skills/vue"]
mode = "ask"
cwd = "~/work"

[profiles.env]
FOO = "bar"
BAZ = "qux"
"#,
        );
        let cfg = load(Some(&p), None).expect("load");
        let full = cfg.profiles.iter().find(|p| p.id == "full").expect("full entry");
        assert_eq!(full.model.as_deref(), Some("claude-opus-4-5"));
        assert_eq!(full.system_prompt.as_deref(), Some("be terse"));
        assert_eq!(
            full.mcps.as_deref(),
            Some(["fs".to_string(), "ripgrep".to_string()].as_slice())
        );
        assert_eq!(
            full.skills.as_deref(),
            Some(
                [
                    PathBuf::from("~/.claude/skills/rust"),
                    PathBuf::from("~/.claude/skills/vue")
                ]
                .as_slice()
            )
        );
        assert_eq!(full.mode.as_deref(), Some("ask"));
        assert_eq!(full.cwd.as_deref(), Some(PathBuf::from("~/work")).as_deref());
        assert_eq!(full.env.get("FOO").map(String::as_str), Some("bar"));
        assert_eq!(full.env.get("BAZ").map(String::as_str), Some("qux"));
        cfg.validate().expect("valid full profile");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn mcps_catalog_parses_full_entry() {
        let p = write_tmp(
            "mcps-full.toml",
            r#"
[[mcps]]
name = "filesystem"
command = "uvx"
args = ["mcp-server-filesystem", "--root", "$HOME"]
scope = "user"

[mcps.env]
LOG_LEVEL = "info"
TOKEN = "abc"
"#,
        );
        let cfg = load(Some(&p), None).expect("load");
        assert_eq!(cfg.mcps.len(), 1);
        let fs_entry = &cfg.mcps[0];
        assert_eq!(fs_entry.name, "filesystem");
        assert_eq!(fs_entry.command, "uvx");
        assert_eq!(
            fs_entry.args,
            vec![
                "mcp-server-filesystem".to_string(),
                "--root".to_string(),
                "$HOME".to_string(),
            ]
        );
        assert_eq!(fs_entry.scope.as_deref(), Some("user"));
        assert_eq!(fs_entry.env.get("LOG_LEVEL").map(String::as_str), Some("info"));
        assert_eq!(fs_entry.env.get("TOKEN").map(String::as_str), Some("abc"));
        cfg.validate().expect("valid catalog");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn mcps_rejects_duplicate_name() {
        let p = write_tmp(
            "mcps-dup.toml",
            r#"
[[mcps]]
name = "fs"
command = "uvx"
args = ["a"]

[[mcps]]
name = "fs"
command = "uvx"
args = ["b"]
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        let err = cfg.validate().expect_err("should reject");
        let msg = err.to_string();
        assert!(msg.contains("duplicate mcp name 'fs'"), "{msg}");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn mcps_rejects_empty_name() {
        let p = write_tmp(
            "mcps-empty.toml",
            r#"
[[mcps]]
name = ""
command = "uvx"
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        let err = cfg.validate().expect_err("should reject");
        let msg = err.to_string();
        assert!(msg.contains("length"), "{msg}");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn mcps_rejects_unknown_field() {
        let p = write_tmp(
            "mcps-unknown.toml",
            r#"
[[mcps]]
name = "fs"
command = "uvx"
bogus = true
"#,
        );
        let err = load(Some(&p), None).expect_err("should reject");
        assert!(err.to_string().contains("failed to parse TOML layer"), "{err}");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn profile_mcps_must_reference_known_catalog_entry() {
        let p = write_tmp(
            "profile-mcps-unknown.toml",
            r#"
[[mcps]]
name = "fs"
command = "uvx"

[[profiles]]
id = "ghost"
agent = "claude-code"
mcps = ["unknown"]
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        let err = cfg.validate().expect_err("should reject");
        let msg = err.to_string();
        assert!(msg.contains("profile 'ghost'"), "{msg}");
        assert!(msg.contains("'unknown'"), "{msg}");
        assert!(msg.contains("Configured names:"), "{msg}");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn profile_mcps_passes_when_every_name_resolves() {
        let p = write_tmp(
            "profile-mcps-ok.toml",
            r#"
[[mcps]]
name = "fs"
command = "uvx"

[[mcps]]
name = "ripgrep"
command = "rg"

[[profiles]]
id = "p"
agent = "claude-code"
mcps = ["fs", "ripgrep"]
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        cfg.validate().expect("all references resolve");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn mcps_user_override_replaces_entry_by_name() {
        // Mirrors the user_agent_entry_overrides_default_by_id pattern.
        // No defaults seed `[[mcps]]`, so we set up two layers manually
        // to exercise merge: a fixture-as-base with `fs` + `ripgrep`,
        // plus a user override that replaces `fs` and appends `git`.
        let base: Config = toml::from_str(
            r#"
[[mcps]]
name = "fs"
command = "old"

[[mcps]]
name = "ripgrep"
command = "rg"
"#,
        )
        .expect("base parses");
        let over: Config = toml::from_str(
            r#"
[[mcps]]
name = "fs"
command = "new"

[[mcps]]
name = "git"
command = "git"
"#,
        )
        .expect("over parses");
        let mut merged = base;
        merged.merge(over);
        let names: Vec<&str> = merged.mcps.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, vec!["fs", "ripgrep", "git"]);
        let fs = merged.mcps.iter().find(|m| m.name == "fs").unwrap();
        assert_eq!(fs.command, "new");
    }

    #[test]
    fn profile_rejects_duplicate_mcp_name() {
        let p = write_tmp(
            "dup-mcp.toml",
            r#"
[[profiles]]
id = "dupe"
agent = "claude-code"
mcps = ["fs", "fs"]
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        let err = cfg.validate().expect_err("should reject");
        let msg = err.to_string();
        assert!(msg.contains("duplicate entry 'fs'"), "{msg}");
        assert!(msg.contains("mcps"), "{msg}");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn profile_rejects_empty_mcp_name() {
        let p = write_tmp(
            "empty-mcp.toml",
            r#"
[[profiles]]
id = "busted"
agent = "claude-code"
mcps = [""]
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        let err = cfg.validate().expect_err("should reject");
        let msg = err.to_string();
        assert!(msg.contains("empty entry"), "{msg}");
        assert!(msg.contains("mcps"), "{msg}");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn profile_absent_mcps_means_all() {
        let p = write_tmp(
            "absent-mcp.toml",
            r#"
[[profiles]]
id = "plain"
agent = "claude-code"
"#,
        );
        let cfg = load(Some(&p), None).expect("load");
        let plain = cfg.profiles.iter().find(|p| p.id == "plain").expect("plain entry");
        assert_eq!(plain.mcps, None, "absent key parses as None");
        assert_eq!(plain.skills, None);
        cfg.validate().expect("absent list validates");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn profile_rejects_duplicate_skills_path() {
        let p = write_tmp(
            "dup-skills.toml",
            r#"
[[profiles]]
id = "dupe"
agent = "claude-code"
skills = ["~/.claude/skills/rust", "~/.claude/skills/rust"]
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        let err = cfg.validate().expect_err("should reject");
        let msg = err.to_string();
        assert!(msg.contains("duplicate entry"), "{msg}");
        assert!(msg.contains("skills"), "{msg}");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn profile_rejects_empty_skills_path() {
        let p = write_tmp(
            "empty-skills.toml",
            r#"
[[profiles]]
id = "busted"
agent = "claude-code"
skills = [""]
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        let err = cfg.validate().expect_err("should reject");
        let msg = err.to_string();
        assert!(msg.contains("empty entry"), "{msg}");
        assert!(msg.contains("skills"), "{msg}");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn profile_empty_mcps_means_none() {
        let p = write_tmp(
            "empty-list-mcp.toml",
            r#"
[[profiles]]
id = "deny"
agent = "claude-code"
mcps = []
skills = []
"#,
        );
        let cfg = load(Some(&p), None).expect("load");
        let deny = cfg.profiles.iter().find(|p| p.id == "deny").expect("deny entry");
        assert_eq!(deny.mcps, Some(vec![]), "empty list parses as Some(vec![])");
        assert_eq!(deny.skills, Some(vec![]));
        cfg.validate().expect("empty list validates");
        fs::remove_file(&p).ok();
    }

    #[test]
    /// With no seeded profiles in `defaults.toml` the merged list is
    /// exactly what the user supplies, in file order. The keyed-list
    /// merge semantics are pinned separately by
    /// `user_agent_entry_overrides_default_by_id`; this test just
    /// confirms that user profiles flow through cleanly.
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
        assert_eq!(ids, vec!["strict", "my-profile"], "user profiles appear in file order");

        let strict = cfg.profiles.iter().find(|p| p.id == "strict").unwrap();
        assert_eq!(strict.agent, "opencode");
        assert_eq!(strict.model.as_deref(), Some("custom-model"));
        assert!(strict.system_prompt.is_none());

        fs::remove_file(&p).ok();
    }
}
