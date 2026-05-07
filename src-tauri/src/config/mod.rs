pub mod agents;
mod autostart;
pub mod daemon;
pub mod extensions;
pub mod keymaps;
pub(crate) mod merge_strategies;
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
pub use agents::{AgentConfig, AgentDefaults, AgentProvider, AgentsConfig, ProfileConfig, ProfileDefaults};
pub use autostart::Autostart;
pub use daemon::{Daemon, Dimension, Edge, Window, WindowMode};
pub use extensions::{McpFile, SkillEntry};
pub use keymaps::{KeymapsConfig, Modifier};
use merge_strategies::{merge_profiles_by_id, overwrite_some};
pub use system_prompt::{SystemPromptEntry, SystemPromptInject};
pub use theme::{Theme, Ui};
use validations::{
    validate_default_profile_id, validate_keymaps_collisions, validate_profile_agent_references, validate_profiles_ids,
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
    /// `[[skills]]` — global skills catalog roots. Each entry is a
    /// directory of `<slug>/SKILL.md` bundles plus an optional
    /// per-entry glob `ignore` array filtering slugs at load time.
    /// Profile-level `skills` wholesale-replaces this default. None
    /// (unset) → defaults seeded by defaults.toml; `Some(vec![])` →
    /// explicit "no skills" override. `~` / env-var expansion at
    /// consume time.
    #[garde(dive)]
    #[merge(strategy = overwrite_some)]
    pub skills: Option<Vec<SkillEntry>>,
    /// `[[mcps]]` — global MCP catalog files. Each entry is a JSON
    /// file in the standard `{ "mcpServers": { ... } }` shape plus an
    /// optional per-entry glob `ignore` array filtering server names
    /// at load time. The loader merges files in iteration order with
    /// later-wins on same-name. Profile-level `mcps` wholesale-
    /// replaces this default. None (unset) → no MCPs; `Some(vec![])`
    /// → explicit empty list. `~` / env-var expansion at consume time.
    #[garde(dive)]
    #[merge(strategy = overwrite_some)]
    pub mcps: Option<Vec<McpFile>>,
    /// Root-level fallback cwd. Used at daemon startup as the chdir
    /// target when `--cwd` isn't passed on the CLI. Mostly useful for
    /// systemd-unit invocations where there's no shell-set cwd. When
    /// neither is set, the daemon inherits the spawning environment's
    /// cwd. `~` / `$VAR` expansion runs at consume time.
    #[garde(skip)]
    #[merge(strategy = overwrite_some)]
    pub cwd: Option<PathBuf>,
    /// `system_prompt` — root-level fallback every profile uses
    /// when its own `system_prompt` isn't set. Array of
    /// `{ file, inject? }` entries, mirroring `[[mcps]]` /
    /// `[[skills]]`. Each entry's `inject.on_create` /
    /// `inject.on_update` independently gates whether that file
    /// rides on Bootstrap::Fresh / Bootstrap::Resume. Profile-level
    /// `system_prompt` wholesale-replaces this default;
    /// `system_prompt = []` is the explicit "no prompt" off-switch.
    #[garde(dive)]
    #[merge(strategy = overwrite_some)]
    pub system_prompt: Option<Vec<SystemPromptEntry>>,
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
    /// flat `ResolvedInstance` at `session/submit` time.
    #[garde(dive)]
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
/// Built from `Config::resolved_skills` at consume time.
#[derive(Debug, Clone)]
pub struct ResolvedSkillEntry {
    pub dir: PathBuf,
    pub ignore: Option<globset::GlobSet>,
}

impl Config {
    /// Resolve every `[[skills]]` entry to an absolute path + compiled
    /// ignore matcher. `~` / env-var expansion via `paths::resolve_user`.
    pub fn resolved_skills(&self) -> Vec<ResolvedSkillEntry> {
        self.skills
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .map(|e| ResolvedSkillEntry {
                dir: crate::paths::resolve_user(&e.dir.to_string_lossy()),
                ignore: e.compile_ignore(),
            })
            .collect()
    }

    /// Resolve every `[[mcps]]` entry to an absolute path + compiled
    /// ignore matcher. Mirrors `resolved_skills`. Profile-level
    /// `mcps` overrides feed through the same shape via
    /// `effective_mcps_for`.
    pub fn resolved_mcps(&self) -> Vec<ResolvedMcpFile> {
        self.mcps
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .map(|e| ResolvedMcpFile {
                file: crate::paths::resolve_user(&e.file.to_string_lossy()),
                ignore: e.compile_ignore(),
            })
            .collect()
    }
}

/// One resolved MCP catalog file. Mirror of `ResolvedSkillEntry` for
/// the `mcps` side.
#[derive(Debug, Clone)]
pub struct ResolvedMcpFile {
    pub file: PathBuf,
    pub ignore: Option<globset::GlobSet>,
}

pub fn load(cli_path: Option<&Path>, profile: Option<&str>) -> Result<Config> {
    tracing::info!(cli_path = ?cli_path, profile = ?profile, "config::load: loading layers");
    let mut layers: Vec<String> = vec![DEFAULTS.to_string()];

    match cli_path {
        Some(p) => {
            if !p.exists() {
                tracing::error!(path = %p.display(), "config::load: cli path missing");
                bail!("config file not found: {}", p.display());
            }
            tracing::debug!(path = %p.display(), "config::load: reading cli-provided layer");
            layers.push(read_layer(p)?);
        }
        None => {
            let default = paths::config_file();
            if default.exists() {
                tracing::debug!(path = %default.display(), "config::load: reading default layer");
                layers.push(read_layer(&default)?);
            }
        }
    }

    if let Some(name) = profile {
        let p = paths::profile_config_file(name);
        if !p.exists() {
            tracing::error!(profile = name, path = %p.display(), "config::load: profile not found");
            bail!("profile '{name}' not found at {}", p.display());
        }
        tracing::debug!(profile = name, path = %p.display(), "config::load: reading profile layer");
        layers.push(read_layer(&p)?);
    }

    let cfg = layers
        .iter()
        .enumerate()
        .try_fold(Config::default(), |mut acc, (idx, body)| -> Result<Config> {
            let layer: Config = toml::from_str(body)
                .map_err(|e| {
                    tracing::error!(layer_index = idx, err = %e, "config::load: TOML parse failed");
                    e
                })
                .context("failed to parse TOML layer")?;
            acc.merge(layer);
            Ok(acc)
        })?;

    tracing::info!(
        layers = layers.len(),
        agents = cfg.agents.agents.len(),
        profiles = cfg.profiles.len(),
        default_agent = ?cfg.agents.agent.default,
        default_profile = ?cfg.profile.default,
        "config::load: layers merged"
    );

    Ok(cfg)
}

fn read_layer(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("failed to read config {}", path.display()))
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

    #[test]
    fn defaults_parse_and_validate() {
        let cfg: Config = toml::from_str(DEFAULTS).expect("defaults must parse");
        cfg.validate().expect("defaults must validate");
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
        assert!(err.to_string().contains("failed to parse TOML layer"));
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
            cfg.validate().unwrap_or_else(|e| panic!("{lvl} validate: {e}"));
            fs::remove_file(&p).ok();
        }
    }

    #[test]
    fn defaults_seed_skills_with_xdg_path() {
        let cfg: Config = toml::from_str(DEFAULTS).expect("defaults must parse");
        let entries = cfg.skills.as_deref().expect("defaults must seed [[skills]]");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].dir, PathBuf::from("~/.config/hyprpilot/skills"));
    }

    #[test]
    fn skills_user_override_replaces_defaults_wholesale() {
        let p = write_tmp(
            "skills-override.toml",
            r#"
[[skills]]
dir = "/opt/skills/team"

[[skills]]
dir = "~/personal/skills"
ignore = ["work-*"]
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        let entries = cfg.skills.as_deref().expect("override applied");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].dir, PathBuf::from("/opt/skills/team"));
        assert_eq!(entries[1].dir, PathBuf::from("~/personal/skills"));
        assert_eq!(entries[1].ignore.as_deref(), Some(&["work-*".to_string()][..]));
        fs::remove_file(&p).ok();
    }

    #[test]
    fn skills_explicit_empty_disables_loading() {
        let p = write_tmp(
            "skills-empty.toml",
            r#"
skills = []
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        assert_eq!(cfg.skills.as_deref(), Some(&[][..]));
        assert!(cfg.resolved_skills().is_empty());
        fs::remove_file(&p).ok();
    }

    #[test]
    fn skills_resolved_expands_tilde() {
        let cfg = Config {
            skills: Some(vec![SkillEntry {
                dir: PathBuf::from("~/.config/hyprpilot/skills"),
                ignore: None,
            }]),
            ..Default::default()
        };
        let resolved = cfg.resolved_skills();
        assert_eq!(resolved.len(), 1);
        let path = resolved[0].dir.to_string_lossy();
        // Tilde expanded to a real home dir; defensive — accept either
        // resolved-form or literal if shellexpand didn't have HOME set.
        assert!(
            path.starts_with('/') || path.contains("hyprpilot/skills"),
            "expected expanded path, got {path}",
        );
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

        cfg.validate().expect("seeded defaults validate");
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
        cfg.validate().expect("cross-scope collisions validate");
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
        cfg.validate().expect("palette vs palette.instances is cross-scope");
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
        cfg.validate().expect("single-char key accepts");
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
        cfg.validate().expect("mixed-order modifiers validate");
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
        cfg.validate().expect("partial override validates");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn root_system_prompt_parses_as_array_of_tables() {
        let p = write_tmp(
            "root-prompt.toml",
            r#"
system_prompt = [
  { file = "~/.config/hyprpilot/prompts/base.md" },
  { file = "~/.config/hyprpilot/prompts/global.md", inject = { on_update = true } },
]
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");

        cfg.validate().expect("root system_prompt validates");
        let prompts = cfg.system_prompt.as_deref().expect("set");
        assert_eq!(prompts.len(), 2);
        assert_eq!(prompts[0].file, PathBuf::from("~/.config/hyprpilot/prompts/base.md"));
        assert!(prompts[0].inject.on_create);
        assert!(!prompts[0].inject.on_update);
        assert_eq!(prompts[1].file, PathBuf::from("~/.config/hyprpilot/prompts/global.md"));
        assert!(
            prompts[1].inject.on_create,
            "on_create stays default-true on partial inject"
        );
        assert!(prompts[1].inject.on_update);
        fs::remove_file(&p).ok();
    }
}
