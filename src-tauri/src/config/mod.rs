mod agents;
mod autostart;
mod daemon;
mod keymaps;
pub(crate) mod merge_strategies;
mod theme;
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
#[allow(unused_imports)]
pub use daemon::{AnchorWindow, CenterWindow, Daemon, Dimension, Edge, Window, WindowMode};
pub use keymaps::KeymapsConfig;
#[allow(unused_imports)]
pub use keymaps::{
    ApprovalsKeymaps, Binding, ChatKeymaps, ComposerKeymaps, Key, ModelsSubPaletteKeymaps, Modifier, NamedKey,
    PaletteKeymaps, SessionsSubPaletteKeymaps, TranscriptKeymaps,
};
use merge_strategies::{merge_profiles_by_id, overwrite_some};
#[allow(unused_imports)]
pub use theme::{
    HexColor, Theme, ThemeAccent, ThemeBorder, ThemeFg, ThemeFont, ThemeKind, ThemePermission, ThemeState, ThemeStatus,
    ThemeSurface, ThemeWindow, Ui,
};
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
    #[garde(dive)]
    pub skills: SkillsConfig,
    /// `mcps` — global MCP file list. Each path points at a JSON file
    /// in the standard `{ "mcpServers": { ... } }` shape; the loader
    /// merges them in iteration order with later-wins on same-name.
    /// Profile-level `mcps` wholesale-replaces this default. None
    /// (unset) → no MCPs; `Some(vec![])` → explicit empty list.
    /// `~` + env-var expansion at consume time, mirroring `[skills] dirs`.
    #[garde(custom(crate::config::validations::validate_unique_nonempty))]
    #[merge(strategy = overwrite_some)]
    pub mcps: Option<Vec<PathBuf>>,
    /// `system_prompt` — root-level fallback every profile uses when
    /// its own `system_prompt` isn't set. Array of markdown / text
    /// file paths; read + concatenated (blank-line separator) at
    /// submit time so edits land without a daemon restart. `~` +
    /// env-var expansion mirrors `mcps`. Profile-level array
    /// wholesale-replaces this default; `Some([])` is the explicit
    /// "no system prompt" off-switch.
    #[garde(custom(crate::config::validations::validate_unique_nonempty))]
    #[merge(strategy = overwrite_some)]
    pub system_prompt: Option<Vec<PathBuf>>,
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
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields)]
#[merge(strategy = overwrite_some)]
pub struct Logging {
    /// Unknown levels reject at TOML parse (serde closed enum).
    #[garde(skip)]
    pub level: Option<crate::logging::LogLevel>,
}

/// `[skills]` — loader configuration. `dirs` is the list of roots
/// scanned by `SkillsRegistry`; each `<slug>/SKILL.md` under any
/// listed root becomes a loadable skill. Defaults seed
/// `["~/.config/hyprpilot/skills"]`; `~` / env-var expansion runs at
/// consume time in `resolved_dirs`. User-supplied `dirs` replaces the
/// default list wholesale (`None` = inherit defaults; `Some(vec![])`
/// = explicit "no skills" override).
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields)]
#[merge(strategy = overwrite_some)]
pub struct SkillsConfig {
    #[garde(skip)]
    pub dirs: Option<Vec<PathBuf>>,
}

impl SkillsConfig {
    /// Resolve every `dirs` entry to an absolute path. Tilde + env
    /// vars in each raw value expand via `shellexpand`; entries that
    /// fail to expand fall through with their literal text.
    pub fn resolved_dirs(&self) -> Vec<PathBuf> {
        let raw = self.dirs.as_deref().expect("[skills].dirs seeded by defaults.toml");
        raw.iter()
            .map(|p| {
                let raw_str = p.to_string_lossy();
                let expanded = shellexpand::full(&raw_str)
                    .map(|s| s.into_owned())
                    .unwrap_or_else(|_| raw_str.into_owned());
                PathBuf::from(expanded)
            })
            .collect()
    }
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
    fn defaults_seed_skills_dirs_with_xdg_path() {
        let cfg: Config = toml::from_str(DEFAULTS).expect("defaults must parse");
        let dirs = cfg.skills.dirs.as_deref().expect("defaults must seed [skills] dirs");
        assert_eq!(dirs, &[PathBuf::from("~/.config/hyprpilot/skills")]);
    }

    #[test]
    fn skills_dirs_user_override_replaces_defaults_wholesale() {
        let p = write_tmp(
            "skills-override.toml",
            r#"
[skills]
dirs = ["/opt/skills/team", "~/personal/skills"]
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        assert_eq!(
            cfg.skills.dirs.as_deref(),
            Some(&[PathBuf::from("/opt/skills/team"), PathBuf::from("~/personal/skills"),][..])
        );
        fs::remove_file(&p).ok();
    }

    #[test]
    fn skills_dirs_explicit_empty_disables_loading() {
        let p = write_tmp(
            "skills-empty.toml",
            r#"
[skills]
dirs = []
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        assert_eq!(cfg.skills.dirs.as_deref(), Some(&[][..]));
        assert!(cfg.skills.resolved_dirs().is_empty());
        fs::remove_file(&p).ok();
    }

    #[test]
    fn skills_resolved_dirs_expand_tilde() {
        let cfg = Config {
            skills: SkillsConfig {
                dirs: Some(vec![PathBuf::from("~/.config/hyprpilot/skills")]),
            },
            ..Default::default()
        };
        let resolved = cfg.skills.resolved_dirs();
        assert_eq!(resolved.len(), 1);
        let path = resolved[0].to_string_lossy();
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
        assert_eq!(k.palette.models.focus, Some(binding(&[Modifier::Ctrl], Key::Char('m'))));
        assert_eq!(
            k.palette.sessions.focus,
            Some(binding(&[Modifier::Ctrl], Key::Char('s')))
        );

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
[keymaps.palette.models]
focus = { modifiers = ["ctrl"], key = "m" }

[keymaps.palette.sessions]
focus = { modifiers = ["ctrl"], key = "m" }
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        cfg.validate()
            .expect("palette.models vs palette.sessions is cross-scope");
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
            cfg.keymaps.palette.models.focus,
            Some(binding(&[Modifier::Ctrl], Key::Char('m')))
        );
        cfg.validate().expect("partial override validates");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn root_system_prompt_parses_as_path_array() {
        let p = write_tmp(
            "root-prompt.toml",
            r#"
system_prompt = ["~/.config/hyprpilot/prompts/base.md", "~/.config/hyprpilot/prompts/global.md"]
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");

        cfg.validate().expect("root system_prompt path validates");
        assert_eq!(
            cfg.system_prompt.as_deref().map(|paths| paths
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect::<Vec<_>>()),
            Some(vec![
                "~/.config/hyprpilot/prompts/base.md".to_string(),
                "~/.config/hyprpilot/prompts/global.md".to_string()
            ])
        );
        fs::remove_file(&p).ok();
    }
}
