pub mod agents;
pub mod extensions;
pub mod mcp;
pub(crate) mod merge_strategies;
pub mod patch;
pub mod system_prompt;
mod validations;
pub(crate) mod with_config;

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use garde::Validate;
use merge::Merge;
use serde::{Deserialize, Serialize};

use crate::paths;
pub use agents::{AgentConfig, AgentProvider, AgentsConfig, ProfileConfig, ProfileDefaults};
pub use extensions::{McpFile, SkillEntry};
pub use mcp::McpConfig;
use merge_strategies::{append_layers, merge_profiles_by_id, overwrite_some};
pub use system_prompt::SystemPromptEntry;
use validations::{
    validate_default_profile_id, validate_profile_agent_references, validate_profiles_ids, validate_profiles_non_empty,
};

pub(crate) const DEFAULTS: &str = include_str!("defaults.toml");

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    #[garde(dive)]
    pub logging: Logging,
    /// `[multiplexer]` — tmux/zellij window-title integration.
    #[garde(dive)]
    pub multiplexer: MultiplexerConfig,
    /// `[[agents]]` at TOML root, flattened here so
    /// `AgentsConfig` stays the single Rust-side unit.
    #[garde(dive)]
    #[serde(flatten)]
    pub agents: AgentsConfig,
    /// `[profile]` — global profile-scope singleton.
    /// `default` is the profile id launched when the positional
    /// `[PROFILE]` argument isn't passed.
    #[garde(dive)]
    #[garde(custom(validate_default_profile_id(&self.profiles)))]
    pub profile: ProfileDefaults,
    /// `[[profiles]]` at TOML root. Each profile binds an agent id to an
    /// optional model override + optional system prompt; resolved into a
    /// flat `ResolvedProfile` at launch time. **At least one
    /// entry is required** — there is no bare-agent fallback. Spawn
    /// picks the positional `[PROFILE]` id first, then `[profile] default`,
    /// then errors when neither resolves to a real profile.
    #[garde(dive)]
    #[garde(custom(validate_profiles_non_empty))]
    #[garde(custom(validate_profiles_ids))]
    #[garde(custom(validate_profile_agent_references(&self.agents.agents)))]
    #[serde(default)]
    #[merge(strategy = merge_profiles_by_id)]
    pub profiles: Vec<ProfileConfig>,
    /// `[[patches]]` — root-level profile patches applied AFTER
    /// profile pick, BEFORE `--with-config` overrides. Each entry
    /// is a partial `ProfileConfig` shape; per-field merge follows
    /// the same `config::patch::merge_values` strategic semantics
    /// the `--with-config` flag already uses (object-field merge,
    /// `$patch: replace` directive, keyed-array merge by id,
    /// primitive-array append+dedupe).
    ///
    /// An optional `$match` sibling filters where the patch applies
    /// before being stripped so it never lands on the profile shape.
    /// `$match.profile = "<glob>"` filters by profile id. Unset
    /// `$match` fields apply to every profile.
    ///
    /// Replaces the older per-field "root fallback" mechanism
    /// (`Config.system_prompt`, `Config.mcps`, `Config.mcp` —
    /// deleted in the same change). Captains who want shared
    /// settings across profiles put them in a (possibly scoped)
    /// patch instead of duplicating per-profile.
    ///
    /// **Additive across config layers** (`append_layers`): a user
    /// config layer's `[[patches]]` EXTENDS the compiled-default seed
    /// (the in-tree `hyprpilot` MCP skills-dir patch) rather than
    /// replacing it — layers concatenate in declaration order, and the
    /// resolve-time fold applies all of them so a later patch can
    /// still override or wipe an earlier one's fields via
    /// `$patch: replace`. This keeps a partial `[patches.mcp]` in a
    /// user layer from silently dropping the seeded skills dir.
    ///
    /// Stored as `Vec<Value>` so the captain's authoring vocabulary
    /// is whatever serde supports on `ProfileConfig` — typed
    /// validation happens AFTER patch application via
    /// `serde_json::from_value::<ProfileConfig>(...) + garde`.
    #[serde(default)]
    #[garde(skip)]
    #[merge(strategy = append_layers)]
    pub patches: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields)]
#[merge(strategy = overwrite_some)]
pub struct Logging {
    /// Tracing filter level. Applied only when neither `--log-level`
    /// nor `RUST_LOG` is set — precedence is `--log-level` >
    /// `RUST_LOG` > `[logging] level` > the `error` default. Unknown
    /// levels reject at TOML parse (serde closed enum). Folded into the
    /// filter in `logging::init`, which `main` calls after `config::load`
    /// so this level takes effect before the first info line.
    #[garde(skip)]
    pub level: Option<crate::logging::LogLevel>,
}

/// `[multiplexer]` — best-effort tmux/zellij window/tab rename on
/// launch. `set_title = true` (seeded by defaults.toml) renames the
/// current tmux window or zellij tab to `hyprpilot@<cwd-basename>`
/// right before `exec()`-ing into the vendor CLI. `false` is the
/// explicit opt-out; outside a multiplexer the feature is a no-op
/// regardless of this flag. See `spawn::multiplexer`.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields)]
#[merge(strategy = overwrite_some)]
pub struct MultiplexerConfig {
    #[garde(skip)]
    pub set_title: Option<bool>,
}

/// One resolved skill catalog entry. `dir` is fully expanded
/// (`~` / `$VAR` collapsed); `ignore` carries the compiled glob
/// matcher (or `None` when the entry has no ignore patterns).
/// `ignore_patterns` stores the raw strings the glob was compiled
/// from so the auto-inject path can forward them to the sidecar's
/// `--skill-ignore` CLI arg without losing the human-readable form.
/// Built from `Config::resolved_skills` at consume time.
#[derive(Debug, Clone)]
pub struct ResolvedSkillEntry {
    pub dir: PathBuf,
    /// Raw glob patterns, preserved alongside the compiled matcher
    /// so they can be serialised to the sidecar's `--skill-ignore`
    /// CLI arg. Mirrors `SkillEntry.ignore` from the TOML shape.
    pub ignore_patterns: Vec<String>,
    pub ignore: Option<globset::GlobSet>,
}

/// One resolved MCP catalog entry. Mirror of `ResolvedSkillEntry` for
/// the `mcps` side. The `source` variant carries either a resolved
/// absolute path (file-on-disk shape) or a pre-extracted inline
/// server map (`mcp_servers` shape); the loader branches on it.
#[derive(Debug, Clone)]
pub struct ResolvedMcpFile {
    pub source: ResolvedMcpSource,
    pub ignore: Option<globset::GlobSet>,
}

/// The two source kinds for an MCP entry. `File` carries the resolved
/// absolute path; `Inline` carries the server map directly so the
/// loader needs no fs round-trip.
#[derive(Debug, Clone)]
pub enum ResolvedMcpSource {
    File(PathBuf),
    Inline(serde_json::Map<String, serde_json::Value>),
}

impl ResolvedMcpFile {
    /// Project one config-shaped `McpFile` onto its runtime form.
    /// Assumes the entry already passed `validate_mcp_source` — the
    /// cross-field check guarantees exactly one of file / inline is
    /// set, so the branch below can't double-match.
    pub fn from_entry(entry: &McpFile) -> Self {
        let source = match (entry.file.as_ref(), entry.mcp_servers.as_ref()) {
            (Some(p), _) => ResolvedMcpSource::File(crate::paths::resolve_user(&p.to_string_lossy())),
            (None, Some(map)) => ResolvedMcpSource::Inline(map.clone()),
            (None, None) => ResolvedMcpSource::Inline(serde_json::Map::new()),
        };
        Self {
            source,
            ignore: entry.compile_ignore(),
        }
    }
}

/// On-disk config format. Picked off the file extension; mirrors
/// the `--with-config` flag's extension set so a captain who
/// authors overlays in YAML can keep their root config in YAML
/// too without juggling formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigFormat {
    Toml,
    Json,
    Yaml,
}

fn format_for_path(path: &Path) -> Result<ConfigFormat> {
    let ext = path.extension().and_then(|s| s.to_str()).map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("toml") => Ok(ConfigFormat::Toml),
        Some("json") => Ok(ConfigFormat::Json),
        Some("yaml" | "yml") => Ok(ConfigFormat::Yaml),
        Some(other) => Err(anyhow!(
            "unsupported config extension '.{other}' at {} — use .toml, .json, .yaml, or .yml",
            path.display()
        )),
        None => Err(anyhow!(
            "config file at {} has no extension — must be .toml, .json, .yaml, or .yml",
            path.display()
        )),
    }
}

/// Parse a config-layer file by extension. Returns the typed `Config`
/// (NOT raw text) — every layer round-trips through the same serde
/// derive, so `deny_unknown_fields` + the closed enums catch typos
/// regardless of format.
fn parse_layer_file(path: &Path) -> Result<Config> {
    let body = fs::read_to_string(path).with_context(|| format!("failed to read config {}", path.display()))?;
    parse_layer_body(&body, format_for_path(path)?, Some(path))
}

fn parse_layer_body(body: &str, format: ConfigFormat, src: Option<&Path>) -> Result<Config> {
    let src_label = src
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "<defaults>".into());
    match format {
        ConfigFormat::Toml => {
            toml::from_str(body).with_context(|| format!("failed to parse TOML config at {src_label}"))
        }
        ConfigFormat::Json => {
            serde_json::from_str(body).with_context(|| format!("failed to parse JSON config at {src_label}"))
        }
        ConfigFormat::Yaml => {
            serde_yaml::from_str(body).with_context(|| format!("failed to parse YAML config at {src_label}"))
        }
    }
}

/// Load + merge the config layers. Runs BEFORE the tracing subscriber
/// is installed (its `[logging] level` feeds the filter), so it stays
/// quiet: failures surface through the returned `Result` and `main`
/// emits the "config: loaded" summary AFTER logging init.
pub fn load(cli_path: Option<&Path>, profile: Option<&str>) -> Result<Config> {
    let mut cfg =
        parse_layer_body(DEFAULTS, ConfigFormat::Toml, None).context("failed to parse compiled defaults.toml")?;

    let root_layer_path: Option<PathBuf> = match cli_path {
        Some(p) => {
            if !p.exists() {
                bail!("config file not found: {}", p.display());
            }
            Some(p.to_path_buf())
        }
        None => paths::find_config_file().context("failed to locate root config")?,
    };

    if let Some(p) = root_layer_path.as_deref() {
        let layer = parse_layer_file(p)?;
        cfg.merge(layer);
    }

    if let Some(name) = profile {
        let resolved = paths::find_profile_config_file(name)
            .with_context(|| format!("failed to locate profile '{name}'"))?
            .ok_or_else(|| {
                anyhow!(
                    "profile '{name}' not found at {} (tried .toml / .json / .yaml / .yml)",
                    paths::profile_config_file(name).display()
                )
            })?;
        let layer = parse_layer_file(&resolved)?;
        cfg.merge(layer);
    }

    Ok(cfg)
}

impl Config {
    /// Run garde's tree walk. Every cross-field rule (agent / profile
    /// / mcp reference checks, non-empty profiles, MCP file-xor-inline)
    /// lives inside the derive walk via higher-order
    /// `custom(fn(&self.x))` hooks — this method is the single
    /// dispatch point that wraps the report.
    pub fn validate(&self) -> Result<()> {
        <Self as Validate>::validate(self).map_err(|report| {
            tracing::error!(%report, "config::validate: garde report");
            anyhow!("config is invalid:\n{report}")
        })?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write;

    use super::*;

    fn write_tmp(name: &str, body: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("hyprpilot-test-{}-{}", std::process::id(), name));
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(body.as_bytes()).unwrap();

        path
    }

    /// Clone the captain's `Config` with a stub `[[profiles]]` entry
    /// injected when the list is empty, so `validate()` doesn't
    /// reject on the "must have at least one profile" rule. Defaults
    /// don't seed a profile anymore (captains supply their own); use
    /// this in tests whose subject is something OTHER than profile
    /// validation (keymap parsing, log-level enums, patches authoring).
    /// Returns a clone so the test can keep reading the original cfg
    /// after running validation against the stub-augmented one.
    fn with_stub_profile(cfg: &Config) -> Config {
        let mut out = cfg.clone();
        if out.profiles.is_empty() {
            out.profiles.push(ProfileConfig {
                id: "stub".into(),
                agent: "claude-code".into(),
                model: None,
                effort: None,
                system_prompt: None,
                mcps: None,
                mcp: None,
                mode: None,
                cwd: None,
                headless: None,
                command: None,
                args: None,
                env: Default::default(),
            });
        }
        out
    }

    #[test]
    fn defaults_parse_but_reject_validate_without_a_profile() {
        // Defaults.toml seeds agents + patches + chrome but
        // intentionally NO profile — captains supply their own so
        // the profile list isn't polluted with a default-pretender.
        // Validation must reject the empty `[[profiles]]` list so a
        // captain who hasn't configured a profile finds out at
        // config-load rather than per-launch.
        let cfg: Config = toml::from_str(DEFAULTS).expect("defaults must parse");
        let err = cfg
            .validate()
            .expect_err("defaults must NOT validate without a profile");
        assert!(
            err.to_string().contains("at least one [[profiles]] entry"),
            "got: {err}"
        );
    }

    #[test]
    fn defaults_seed_multiplexer_set_title() {
        // Pins the leaf `spawn::launch_profile` `.expect()`s at
        // spawn time — a captain deleting `[multiplexer]` from
        // defaults.toml must fail here, not panic at launch.
        let cfg: Config = toml::from_str(DEFAULTS).expect("defaults must parse");
        assert_eq!(cfg.multiplexer.set_title, Some(true));
    }

    #[test]
    fn defaults_do_not_seed_logging_level() {
        // K-750 item 1: seeding `[logging] level = "info"` made the
        // code fallback `warn,hyprpilot=info` unreachable, so rmcp /
        // tokio logged at a flat global `info`. The seed is removed so
        // an unset `logging.level` lets that scoped fallback own the
        // default. Pin the absence so a future re-seed surfaces here.
        let cfg: Config = toml::from_str(DEFAULTS).expect("defaults must parse");
        assert_eq!(cfg.logging.level, None, "defaults must NOT seed logging.level");
    }

    #[test]
    fn defaults_seed_mcp_via_root_patch() {
        // `[mcp]` is no longer a root field — it lives on
        // `ProfileConfig` and gets seeded via the default
        // `[[patches]]` entry. The seed carries ONLY the XDG skills
        // dir (the load-bearing value that must survive additive
        // layer merge); `enabled` / `autoAcceptTools` /
        // `autoRejectTools` are the typed `McpConfig::default()` the
        // resolver backfills, so they are intentionally absent here.
        // Pin the seeded skills shape so a future captain removing the
        // entry surfaces here instead of breaking auto-inject silently.
        let cfg: Config = toml::from_str(DEFAULTS).expect("defaults must parse");
        let patches = cfg.patches.as_deref().expect("defaults must seed [[patches]]");
        assert!(!patches.is_empty(), "defaults must seed at least one root patch");

        let mcp_patch = patches
            .iter()
            .find_map(|p| p.as_object()?.get("mcp"))
            .expect("a default patch must carry an mcp field");
        let m = mcp_patch.as_object().expect("mcp patch is an object");
        assert_eq!(
            m.get("enabled"),
            None,
            "mcp.enabled is single-sourced in McpConfig::default(), not the seed"
        );
        assert_eq!(
            m.get("autoAcceptTools"),
            None,
            "mcp.autoAcceptTools is single-sourced in McpConfig::default(), not the seed"
        );
        let skills = m
            .get("skills")
            .and_then(|s| s.get("roots"))
            .and_then(serde_json::Value::as_array)
            .expect("default mcp.skills.roots");
        assert_eq!(skills.len(), 1, "default mcp.skills.roots seeds exactly the XDG dir");
        assert_eq!(
            skills[0].get("dir").and_then(serde_json::Value::as_str),
            Some("~/.config/hyprpilot/skills")
        );
    }

    #[test]
    fn load_merges_cli_path_over_defaults() {
        let p = write_tmp(
            "merge.toml",
            r#"
[logging]
level = "debug"
"#,
        );
        let cfg = load(Some(&p), None).expect("load");
        assert_eq!(cfg.logging.level, Some(crate::logging::LogLevel::Debug));
        fs::remove_file(&p).ok();
    }

    #[test]
    fn load_errors_when_cli_path_missing() {
        let missing = PathBuf::from("/nonexistent/hyprpilot-test-never.toml");
        let err = load(Some(&missing), None).expect_err("should error");
        assert!(err.to_string().contains("config file not found"));
    }

    #[test]
    fn load_rejects_unknown_fields() {
        let p = write_tmp("unknown.toml", "bogus = true\n");
        let err = load(Some(&p), None).expect_err("should error");
        let msg = format!("{err:#}");
        assert!(msg.contains("failed to parse TOML config"), "got: {msg}");
        fs::remove_file(&p).ok();
    }

    /// `--config <path>.json` loads + merges a JSON-authored root
    /// config. The fields shipped on the wire are the same as TOML —
    /// the file format is the only thing that changes.
    #[test]
    fn load_accepts_json_extension() {
        let p = write_tmp("json-cli.json", r#"{"logging": {"level": "debug"}}"#);
        let cfg = load(Some(&p), None).expect("load json");
        assert_eq!(cfg.logging.level, Some(crate::logging::LogLevel::Debug));
        fs::remove_file(&p).ok();
    }

    /// `--config <path>.yaml` loads + merges a YAML-authored root
    /// config. `.yml` works too via the same extension matcher.
    #[test]
    fn load_accepts_yaml_extension() {
        let p = write_tmp("yaml-cli.yaml", "logging:\n  level: debug\n");
        let cfg = load(Some(&p), None).expect("load yaml");
        assert_eq!(cfg.logging.level, Some(crate::logging::LogLevel::Debug));
        fs::remove_file(&p).ok();
    }

    /// `deny_unknown_fields` survives the JSON parse path too — a
    /// stray field at the top level rejects with a parse error
    /// instead of silently dropping.
    #[test]
    fn load_rejects_unknown_fields_in_json() {
        let p = write_tmp("unknown.json", r#"{"bogus": true}"#);
        let err = load(Some(&p), None).expect_err("should error");
        let msg = format!("{err:#}");
        assert!(msg.contains("failed to parse JSON config"), "got: {msg}");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn load_rejects_unknown_fields_in_yaml() {
        let p = write_tmp("unknown.yaml", "bogus: true\n");
        let err = load(Some(&p), None).expect_err("should error");
        let msg = format!("{err:#}");
        assert!(msg.contains("failed to parse YAML config"), "got: {msg}");
        fs::remove_file(&p).ok();
    }

    /// Captain points `--config` at a file with an unsupported
    /// extension → clear error naming the extension, NOT a confusing
    /// TOML parse failure.
    #[test]
    fn load_rejects_unsupported_extension() {
        let p = write_tmp("unsupported.ini", "ignored\n");
        let err = load(Some(&p), None).expect_err("should error");
        let msg = format!("{err:#}");
        assert!(msg.contains("unsupported config extension"), "got: {msg}");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn toml_rejects_bad_log_level() {
        // With `LogLevel` as a closed enum, unknown levels fail at
        // TOML parse time rather than at validate time. anyhow's
        // top-level message is "failed to parse TOML layer"; the
        // serde detail lives in the underlying source.
        let p = write_tmp(
            "bad-level.toml",
            r#"
[logging]
level = "verbose"
"#,
        );
        let err = load(Some(&p), None).expect_err("should error on parse");
        let chain = err.chain().map(|e| e.to_string()).collect::<Vec<_>>().join("\n");
        assert!(chain.contains("verbose") || chain.contains("level"), "{chain}");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn toml_accepts_known_levels() {
        for lvl in ["trace", "debug", "info", "warn", "error"] {
            let body = format!(
                r#"
[logging]
level = "{lvl}"
"#
            );
            let p = write_tmp(&format!("level-{lvl}.toml"), &body);
            let cfg = load(Some(&p), None).unwrap_or_else(|e| panic!("{lvl} parse: {e}"));
            with_stub_profile(&cfg)
                .validate()
                .unwrap_or_else(|e| panic!("{lvl} validate: {e}"));
            fs::remove_file(&p).ok();
        }
    }

    /// Root-level `[[patches]]` array-of-tables parses + carries the
    /// captain's `ProfileConfig`-shaped overlay (`system_prompt`,
    /// `mcps`, `mcp.skills`, `env`, …) through to the resolver.
    /// Mirrors the deleted `root_system_prompt_parses_as_array_of_tables`
    /// — same authoring vocabulary, now wrapped under a patch.
    #[test]
    fn root_patches_parse_as_array_of_tables() {
        let p = write_tmp(
            "root-patches.toml",
            r#"
[[patches]]
[[patches.system_prompt]]
file = "~/.config/hyprpilot/prompts/base.md"

[[patches.system_prompt]]
file = "~/.config/hyprpilot/prompts/global.md"
inject = false
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        with_stub_profile(&cfg).validate().expect("root patches validate");

        let patches = cfg.patches.as_deref().expect("set");
        assert!(!patches.is_empty(), "captain's patch must merge in");

        // Find the patch carrying system_prompt (default seed also
        // contributes one, so iterate instead of indexing).
        let prompt_patch = patches
            .iter()
            .find_map(|p| p.as_object()?.get("system_prompt")?.as_array())
            .expect("captain's patch declares system_prompt");
        assert_eq!(prompt_patch.len(), 2);
        assert_eq!(
            prompt_patch[0].get("file").and_then(serde_json::Value::as_str),
            Some("~/.config/hyprpilot/prompts/base.md")
        );
        fs::remove_file(&p).ok();
    }

    /// Cross-layer patch merge (K-746): a user config layer's
    /// `[[patches]]` EXTENDS the defaults-seeded patch instead of
    /// wholesale-replacing it. A partial `[patches.mcp]` in the user
    /// layer (tightening the accept globs, no `skills`) must NOT drop
    /// the seeded skills dir — both patches survive the layer merge
    /// and fold in declaration order at resolve time. This is the
    /// footgun the `append_layers` strategy fixes; under the old
    /// `overwrite_some` the user layer clobbered the seed and the
    /// skills dir silently vanished.
    #[test]
    fn user_layer_patches_extend_the_defaults_seed() {
        let p = write_tmp(
            "additive-patches.toml",
            r#"
[[profiles]]
id = "engineer"
agent = "claude-code"

[[patches]]
[patches.mcp]
autoAcceptTools = ["read_*"]
"#,
        );
        let cfg = load(Some(&p), None).expect("load");
        fs::remove_file(&p).ok();

        // Both the defaults-seeded patch AND the user layer's patch
        // survive the layer merge (append, not overwrite).
        let patches = cfg.patches.as_deref().expect("patches set");
        assert_eq!(patches.len(), 2, "seed + user-layer patch both present");

        // Resolve the profile: the two patches fold in order — the
        // seed's skills dir first, then the user's tighter accept
        // globs on top.
        let patched = crate::resolve::resolve_effective_profile(&cfg, Some("engineer"), &[]).expect("resolve");
        let mcp = patched.mcp.as_ref().expect("profile carries a folded mcp block");

        // Seed's skills dir survived the partial user patch (the footgun).
        let skills = mcp
            .skills
            .as_ref()
            .and_then(|s| s.roots.as_deref())
            .expect("seeded skills roots preserved");
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].dir, PathBuf::from("~/.config/hyprpilot/skills"));

        // User layer's override folded on top.
        assert_eq!(
            mcp.auto_accept_tools.as_deref(),
            Some(["read_*".to_string()].as_slice())
        );

        // The effective (backfilled) block carries the default value
        // leaves the user patch never mentioned, and the seeded skills
        // still flow through to the launch.
        let effective = crate::resolve::effective_mcp_with(&patched);
        assert!(effective.enabled(), "enabled backfilled from McpConfig::default()");
        assert_eq!(effective.auto_accept_tools(), ["read_*".to_string()]);
        assert_eq!(effective.resolved_skills().len(), 1, "seeded skills flow to the launch");
    }
}
