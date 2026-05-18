pub mod agents;
mod autostart;
pub mod daemon;
pub mod extensions;
pub mod keymaps;
pub mod mcp;
pub(crate) mod merge_strategies;
pub mod patch;
pub mod remote;
pub mod system_prompt;
pub mod theme;
mod validations;

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use garde::Validate;
use merge::Merge;
use serde::{Deserialize, Serialize};

use crate::paths;
pub use agents::{AgentConfig, AgentProvider, AgentsConfig, ProfileConfig, ProfileDefaults};
pub use autostart::Autostart;
pub use daemon::{Daemon, Dimension, Edge, Window, WindowMode};
pub use extensions::{McpFile, SkillEntry};
pub use keymaps::{KeymapsConfig, Modifier};
pub use mcp::McpConfig;
use merge_strategies::{merge_profiles_by_id, overwrite_some};
pub use remote::RemoteConfig;
pub use system_prompt::{SystemPromptEntry, SystemPromptInject};
pub use theme::{Theme, Ui};
use validations::{
    validate_default_profile_id, validate_keymaps_collisions, validate_profile_agent_references, validate_profiles_ids,
    validate_profiles_non_empty,
};

pub(crate) const DEFAULTS: &str = include_str!("defaults.toml");

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    #[garde(dive)]
    pub daemon: Daemon,
    /// `[autostart]` — top-level. Drives the boot-time reconcile
    /// against `tauri-plugin-autostart`. Top-level (not nested under
    /// `[daemon]`) because autostart is a property of the binary's
    /// relationship to the OS, not of the daemon's internal config.
    #[garde(dive)]
    pub autostart: Autostart,
    #[garde(dive)]
    pub logging: Logging,
    #[garde(dive)]
    pub ui: Ui,
    /// `[[agents]]` + `[agent]` at TOML root, flattened here so
    /// `AgentsConfig` stays the single Rust-side unit.
    #[garde(dive)]
    #[serde(flatten)]
    pub agents: AgentsConfig,
    /// `[profile]` — global profile-scope singleton (mirrors `[agent]`).
    /// `default` is the profile id used when `submit` doesn't carry
    /// one and the wire / palette doesn't pre-select.
    #[garde(dive)]
    #[garde(custom(validate_default_profile_id(&self.profiles)))]
    pub profile: ProfileDefaults,
    /// `[[profiles]]` at TOML root. Each profile binds an agent id to an
    /// optional model override + optional system prompt; resolved into a
    /// flat `ResolvedInstance` at `session/submit` time. **At least one
    /// entry is required** — there is no bare-agent fallback. Spawn
    /// picks `--profile <id>` first, then `[profile] default`, then
    /// errors when neither resolves to a real profile.
    #[garde(dive)]
    #[garde(custom(validate_profiles_non_empty))]
    #[garde(custom(validate_profiles_ids))]
    #[garde(custom(validate_profile_agent_references(&self.agents.agents)))]
    #[serde(default)]
    #[merge(strategy = merge_profiles_by_id)]
    pub profiles: Vec<ProfileConfig>,
    /// Overlay-wide keyboard bindings. Structured group-per-UI-surface
    /// (chat / approvals / composer / palette / transcript); palette
    /// carries nested subgroups (`models`, `sessions`) as their own
    /// collision scopes. Every leaf is a binding string parsed by the
    /// UI's `parseKeys` grammar; collisions inside a scope reject at
    /// load time, cross-scope collisions are fine.
    #[garde(dive)]
    #[garde(custom(validate_keymaps_collisions))]
    pub keymaps: KeymapsConfig,
    /// `[completion]` — composer autocomplete tuning. The `ripgrep`
    /// subgroup controls whether the in-process ripgrep source fires
    /// on every keystroke (auto, with debounce) or only on manual
    /// trigger (Tab / Ctrl+Space).
    #[garde(dive)]
    pub completion: CompletionConfig,
    /// `[remote]` — TLS axum HTTP+WS server alongside the Tauri
    /// overlay. Off by default. When enabled, lets a phone (or any
    /// browser on the LAN) load the same Vue overlay. Per-connection
    /// pair confirmation; no persistent tokens.
    #[garde(dive)]
    pub remote: RemoteConfig,
    /// `[[patches]]` — root-level profile patches applied AFTER
    /// profile pick, BEFORE `--with-config` overrides. Each entry
    /// is a partial `ProfileConfig` shape; per-field merge follows
    /// the same `config::patch::merge_values` strategic semantics
    /// the `--with-config` flag already uses (object-field merge,
    /// `$patch: replace` directive, keyed-array merge by id,
    /// primitive-array append+dedupe).
    ///
    /// An optional `$match: { profile: "<glob>" }` sibling at the
    /// top of each patch object filters which profiles the patch
    /// applies to (stripped before merging so it never lands on the
    /// profile shape). Unset `$match` = applies to every profile.
    ///
    /// Replaces the older per-field "root fallback" mechanism
    /// (`Config.system_prompt`, `Config.mcps`, `Config.mcp` —
    /// deleted in the same change). Captains who want shared
    /// settings across profiles put them in a (possibly scoped)
    /// patch instead of duplicating per-profile.
    ///
    /// Stored as `Vec<Value>` so the captain's authoring vocabulary
    /// is whatever serde supports on `ProfileConfig` — typed
    /// validation happens AFTER patch application via
    /// `serde_json::from_value::<ProfileConfig>(...) + garde`.
    #[serde(default)]
    #[garde(skip)]
    #[merge(strategy = overwrite_some)]
    pub patches: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields)]
pub struct CompletionConfig {
    #[garde(dive)]
    pub ripgrep: RipgrepCompletionConfig,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields)]
#[merge(strategy = overwrite_some)]
pub struct RipgrepCompletionConfig {
    /// Auto-trigger ripgrep on plain text input (no manual gate).
    /// `true` means typing past `min_prefix` characters fires a
    /// ripgrep query through the standard debounce; `false` keeps
    /// ripgrep manual-only (Tab / Ctrl+Space).
    #[garde(skip)]
    pub auto: Option<bool>,
    /// Debounce applied UI-side before firing the auto-trigger
    /// query. Bumped from the global default because ripgrep walks
    /// the cwd's file tree and is heavier than the path / skills
    /// sources.
    #[garde(range(min = 0, max = 5_000))]
    pub debounce_ms: Option<u32>,
    /// Minimum token length before ripgrep claims the query at all.
    /// Single letters return thousands of matches and burn CPU.
    #[garde(range(min = 1, max = 64))]
    pub min_prefix: Option<usize>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields)]
#[merge(strategy = overwrite_some)]
pub struct Logging {
    /// Unknown levels reject at TOML parse (serde closed enum).
    #[garde(skip)]
    pub level: Option<crate::logging::LogLevel>,
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

pub fn load(cli_path: Option<&Path>, profile: Option<&str>) -> Result<Config> {
    tracing::info!(cli_path = ?cli_path, profile = ?profile, "config::load: loading layers");

    let mut cfg =
        parse_layer_body(DEFAULTS, ConfigFormat::Toml, None).context("failed to parse compiled defaults.toml")?;

    let root_layer_path: Option<PathBuf> = match cli_path {
        Some(p) => {
            if !p.exists() {
                tracing::error!(path = %p.display(), "config::load: cli path missing");
                bail!("config file not found: {}", p.display());
            }
            Some(p.to_path_buf())
        }
        None => paths::find_config_file().context("failed to locate root config")?,
    };

    if let Some(p) = root_layer_path.as_deref() {
        tracing::debug!(path = %p.display(), "config::load: reading root layer");
        let layer = parse_layer_file(p)?;
        cfg.merge(layer);
    }

    if let Some(name) = profile {
        let resolved = paths::find_profile_config_file(name)
            .with_context(|| format!("failed to locate profile '{name}'"))?
            .ok_or_else(|| {
                tracing::error!(profile = name, "config::load: profile not found");
                anyhow!(
                    "profile '{name}' not found at {} (tried .toml / .json / .yaml / .yml)",
                    paths::profile_config_file(name).display()
                )
            })?;
        tracing::debug!(profile = name, path = %resolved.display(), "config::load: reading profile layer");
        let layer = parse_layer_file(&resolved)?;
        cfg.merge(layer);
    }

    tracing::info!(
        root_layer = ?root_layer_path,
        profile = ?profile,
        agents = cfg.agents.agents.len(),
        profiles = cfg.profiles.len(),
        default_profile = ?cfg.profile.default,
        "config::load: layers merged"
    );

    Ok(cfg)
}

impl Config {
    /// Run garde's tree walk. Every cross-field rule (keymaps
    /// collisions, profile prompt-source exclusivity, agent /
    /// profile / mcp reference checks) lives inside the derive
    /// walk via higher-order `custom(fn(&self.x))` hooks — this
    /// method is the single dispatch point that wraps the report.
    pub fn validate(&self) -> Result<()> {
        <Self as Validate>::validate(self).map_err(|report| {
            tracing::error!(%report, "config::validate: garde report");
            anyhow!("config is invalid:\n{report}")
        })?;
        tracing::debug!("config::validate: config validated");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write;

    use super::*;
    use crate::config::keymaps::{Binding, Key, NamedKey};

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
                system_prompt: None,
                mcps: None,
                mcp: None,
                mode: None,
                cwd: None,
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
        // daemon boot rather than per-spawn.
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
    fn defaults_seed_mcp_via_root_patch() {
        // `[mcp]` is no longer a root field — it lives on
        // `ProfileConfig` and gets seeded via the default
        // `[[patches]]` entry. Pin the seeded patch shape so a
        // future captain removing the entry surfaces here instead
        // of breaking auto-inject silently at runtime.
        let cfg: Config = toml::from_str(DEFAULTS).expect("defaults must parse");
        let patches = cfg.patches.as_deref().expect("defaults must seed [[patches]]");
        assert!(!patches.is_empty(), "defaults must seed at least one root patch");

        let mcp_patch = patches
            .iter()
            .find_map(|p| p.as_object()?.get("mcp"))
            .expect("a default patch must carry an mcp field");
        let m = mcp_patch.as_object().expect("mcp patch is an object");
        assert_eq!(m.get("enabled").and_then(serde_json::Value::as_bool), Some(true));
        assert_eq!(
            m.get("autoAcceptTools").and_then(serde_json::Value::as_array),
            Some(&vec![serde_json::Value::String("*".into())]),
            "default mcp.autoAcceptTools must be [\"*\"]",
        );
        let skills = m
            .get("skills")
            .and_then(serde_json::Value::as_array)
            .expect("default mcp.skills");
        assert_eq!(skills.len(), 1, "default mcp.skills seeds exactly the XDG dir");
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

    fn binding(mods: &[Modifier], key: Key) -> Binding {
        Binding {
            modifiers: mods.to_vec(),
            key,
        }
    }

    #[test]
    fn defaults_populate_every_keymap_leaf() {
        let cfg: Config = toml::from_str(DEFAULTS).expect("defaults must parse");
        let k = &cfg.keymaps;

        assert_eq!(k.chat.submit, Some(binding(&[], Key::Named(NamedKey::Enter))));
        assert_eq!(
            k.chat.newline,
            Some(binding(&[Modifier::Shift], Key::Named(NamedKey::Enter)))
        );
        assert_eq!(k.approvals.allow, Some(binding(&[Modifier::Ctrl], Key::Char('g'))));
        assert_eq!(k.approvals.deny, Some(binding(&[Modifier::Ctrl], Key::Char('r'))));
        assert_eq!(
            k.queue.send,
            Some(binding(&[Modifier::Ctrl], Key::Named(NamedKey::Enter)))
        );
        assert_eq!(
            k.queue.drop,
            Some(binding(&[Modifier::Ctrl], Key::Named(NamedKey::Backspace)))
        );
        assert_eq!(k.composer.paste, Some(binding(&[Modifier::Ctrl], Key::Char('p'))));
        assert_eq!(k.composer.tab_completion, Some(binding(&[], Key::Named(NamedKey::Tab))));
        assert_eq!(
            k.composer.shift_tab,
            Some(binding(&[Modifier::Shift], Key::Named(NamedKey::Tab)))
        );
        assert_eq!(
            k.composer.history_up,
            Some(binding(&[Modifier::Ctrl], Key::Named(NamedKey::ArrowUp)))
        );
        assert_eq!(
            k.composer.history_down,
            Some(binding(&[Modifier::Ctrl], Key::Named(NamedKey::ArrowDown)))
        );
        assert_eq!(k.palette.open, Some(binding(&[Modifier::Ctrl], Key::Char('k'))));
        assert_eq!(k.palette.close, Some(binding(&[], Key::Named(NamedKey::Escape))));
        assert_eq!(
            k.palette.instances.focus,
            Some(binding(&[Modifier::Ctrl], Key::Char('i')))
        );
        assert_eq!(k.chat.cancel_turn, Some(binding(&[Modifier::Ctrl], Key::Char('d'))));
        assert_eq!(k.chat.focus_input, Some(binding(&[Modifier::Ctrl], Key::Char('f'))));

        with_stub_profile(&cfg).validate().expect("seeded defaults validate");
    }

    #[test]
    fn keymaps_validate_rejects_same_scope_collision() {
        let p = write_tmp(
            "keymap-collision.toml",
            r#"
[keymaps.composer]
paste = { key = "tab" }
tab_completion = { key = "tab" }
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        let err = cfg.validate().expect_err("should reject within-scope collision");
        let msg = err.to_string();
        assert!(msg.contains("keymaps.composer"), "{msg}");
        assert!(msg.contains("tab"), "{msg}");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn keymaps_validate_allows_cross_scope_collision() {
        // chat.submit == palette.open — different scopes, OK.
        let p = write_tmp(
            "keymap-cross.toml",
            r#"
[keymaps.chat]
submit = { key = "enter" }

[keymaps.palette]
open = { key = "enter" }
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        with_stub_profile(&cfg)
            .validate()
            .expect("cross-scope collisions validate");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn keymaps_validate_allows_cross_subgroup_collision() {
        let p = write_tmp(
            "keymap-subgroup.toml",
            r#"
[keymaps.palette]
open = { modifiers = ["ctrl"], key = "i" }

[keymaps.palette.instances]
focus = { modifiers = ["ctrl"], key = "i" }
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        with_stub_profile(&cfg)
            .validate()
            .expect("palette vs palette.instances is cross-scope");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn binding_rejects_unknown_modifier() {
        let p = write_tmp(
            "keymap-mod.toml",
            r#"
[keymaps.chat]
submit = { modifiers = ["hyper"], key = "k" }
"#,
        );
        let err = load(Some(&p), None).expect_err("unknown modifier rejects at parse");
        let chain = err.chain().map(|e| e.to_string()).collect::<Vec<_>>().join("\n");
        assert!(chain.contains("hyper") || chain.contains("variant"), "{chain}");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn binding_rejects_unknown_named_key() {
        let p = write_tmp(
            "keymap-bad-key.toml",
            r#"
[keymaps.chat]
submit = { key = "return" }
"#,
        );
        let err = load(Some(&p), None).expect_err("unknown named key rejects at parse");
        let chain = err.chain().map(|e| e.to_string()).collect::<Vec<_>>().join("\n");
        assert!(chain.contains("return"), "{chain}");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn binding_rejects_duplicate_modifiers() {
        let p = write_tmp(
            "keymap-dup-mod.toml",
            r#"
[keymaps.chat]
submit = { modifiers = ["ctrl", "ctrl"], key = "k" }
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        let err = cfg.validate().expect_err("duplicate modifier rejects");
        assert!(err.to_string().contains("duplicate modifier"), "{err}");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn binding_accepts_single_char_key() {
        let p = write_tmp(
            "keymap-char.toml",
            r#"
[keymaps.palette]
open = { key = "?" }
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        with_stub_profile(&cfg).validate().expect("single-char key accepts");
        assert_eq!(cfg.keymaps.palette.open, Some(binding(&[], Key::Char('?'))));
        fs::remove_file(&p).ok();
    }

    #[test]
    fn binding_canonicalises_modifier_order() {
        let p = write_tmp(
            "keymap-order.toml",
            r#"
[keymaps.chat]
submit = { modifiers = ["shift", "ctrl"], key = "enter" }
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        with_stub_profile(&cfg)
            .validate()
            .expect("mixed-order modifiers validate");
        // Source order ["shift","ctrl"] canonicalises to sorted ascending.
        let submit = cfg.keymaps.chat.submit.expect("seeded");
        assert_eq!(submit.modifiers, vec![Modifier::Ctrl, Modifier::Shift]);
        fs::remove_file(&p).ok();
    }

    #[test]
    fn keymaps_partial_override_preserves_untouched_leaves() {
        let p = write_tmp(
            "keymap-partial.toml",
            r#"
[keymaps.chat]
submit = { modifiers = ["ctrl"], key = "enter" }
"#,
        );
        let cfg = load(Some(&p), None).expect("load");
        // Overridden leaf.
        assert_eq!(
            cfg.keymaps.chat.submit,
            Some(binding(&[Modifier::Ctrl], Key::Named(NamedKey::Enter)))
        );
        // Same-group untouched leaf falls through.
        assert_eq!(
            cfg.keymaps.chat.newline,
            Some(binding(&[Modifier::Shift], Key::Named(NamedKey::Enter)))
        );
        // Other groups untouched.
        assert_eq!(
            cfg.keymaps.approvals.allow,
            Some(binding(&[Modifier::Ctrl], Key::Char('g')))
        );
        assert_eq!(
            cfg.keymaps.approvals.deny,
            Some(binding(&[Modifier::Ctrl], Key::Char('r')))
        );
        assert_eq!(
            cfg.keymaps.palette.open,
            Some(binding(&[Modifier::Ctrl], Key::Char('k')))
        );
        assert_eq!(
            cfg.keymaps.palette.instances.focus,
            Some(binding(&[Modifier::Ctrl], Key::Char('i')))
        );
        with_stub_profile(&cfg).validate().expect("partial override validates");
        fs::remove_file(&p).ok();
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
inject = { on_update = true }
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
}
