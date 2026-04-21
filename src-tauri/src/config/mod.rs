use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use garde::Validate;
use serde::{Deserialize, Serialize};

use crate::paths;

const DEFAULTS: &str = include_str!("defaults.toml");

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    #[garde(dive)]
    pub daemon: Daemon,
    #[garde(dive)]
    pub logging: Logging,
    #[garde(dive)]
    pub ui: Ui,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct Daemon {
    #[garde(skip)]
    pub socket: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct Logging {
    #[garde(inner(custom(validate_log_level)))]
    pub level: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct Ui {
    #[garde(dive)]
    pub theme: Theme,
}

/// Palette tokens surfaced to the webview as CSS custom properties. Each
/// leaf is `Option<String>` so partial overrides in user TOML compose
/// cleanly over the compiled defaults layer — `merge_theme` walks the tree
/// field-by-field using `or`. Leaf naming is consistent across groups:
/// `default` is the base value; siblings are variants.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct Theme {
    #[garde(dive)]
    pub font: ThemeFont,
    #[garde(dive)]
    pub window: ThemeWindow,
    #[garde(dive)]
    pub surface: ThemeSurface,
    #[garde(dive)]
    pub fg: ThemeFg,
    #[garde(dive)]
    pub border: ThemeBorder,
    #[garde(dive)]
    pub accent: ThemeAccent,
    #[garde(dive)]
    pub state: ThemeState,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeFont {
    #[garde(inner(length(min = 1)))]
    pub family: Option<String>,
}

/// The outer container — everything intrinsic to the window frame. `default`
/// is the window's background fill; `edge` is the accent stripe on the
/// left edge.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeWindow {
    #[garde(inner(custom(validate_hex_color)))]
    pub default: Option<String>,
    #[garde(inner(custom(validate_hex_color)))]
    pub edge: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeSurface {
    #[garde(dive)]
    pub card: SurfaceCard,
    #[garde(inner(custom(validate_hex_color)))]
    pub compose: Option<String>,
    #[garde(inner(custom(validate_hex_color)))]
    pub text: Option<String>,
}

/// Cards carry messages — the palette splits them by speaker so user and
/// assistant cards can diverge in bg (and future accent, border, fg…)
/// without needing two disjoint config trees.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct SurfaceCard {
    #[garde(dive)]
    pub user: Card,
    #[garde(dive)]
    pub assistant: Card,
}

/// A single card's painted tokens. `bg` is the base paint; future fields
/// (accent stripe, border, text-on-card) slot in alongside without a
/// schema rewrite.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct Card {
    #[garde(inner(custom(validate_hex_color)))]
    pub bg: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeFg {
    #[garde(inner(custom(validate_hex_color)))]
    pub default: Option<String>,
    #[garde(inner(custom(validate_hex_color)))]
    pub dim: Option<String>,
    #[garde(inner(custom(validate_hex_color)))]
    pub muted: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeBorder {
    #[garde(inner(custom(validate_hex_color)))]
    pub default: Option<String>,
    #[garde(inner(custom(validate_hex_color)))]
    pub soft: Option<String>,
    #[garde(inner(custom(validate_hex_color)))]
    pub focus: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeAccent {
    #[garde(inner(custom(validate_hex_color)))]
    pub default: Option<String>,
    #[garde(inner(custom(validate_hex_color)))]
    pub user: Option<String>,
    #[garde(inner(custom(validate_hex_color)))]
    pub assistant: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeState {
    #[garde(inner(custom(validate_hex_color)))]
    pub idle: Option<String>,
    #[garde(inner(custom(validate_hex_color)))]
    pub stream: Option<String>,
    #[garde(inner(custom(validate_hex_color)))]
    pub pending: Option<String>,
    #[garde(inner(custom(validate_hex_color)))]
    pub awaiting: Option<String>,
}

pub fn load(cli_path: Option<&Path>, profile: Option<&str>) -> Result<Config> {
    let mut layers: Vec<String> = vec![DEFAULTS.to_string()];

    match cli_path {
        Some(p) => {
            if !p.exists() {
                bail!("config file not found: {}", p.display());
            }
            layers.push(read_layer(p)?);
        }
        None => {
            let default = paths::config_file();
            if default.exists() {
                layers.push(read_layer(&default)?);
            }
        }
    }

    if let Some(name) = profile {
        let p = paths::profile_config_file(name);
        if !p.exists() {
            bail!("profile '{name}' not found at {}", p.display());
        }
        layers.push(read_layer(&p)?);
    }

    layers.iter().try_fold(Config::default(), |acc, body| {
        let layer: Config = toml::from_str(body).context("failed to parse TOML layer")?;

        Ok(Config {
            daemon: Daemon {
                socket: layer.daemon.socket.or(acc.daemon.socket),
            },
            logging: Logging {
                level: layer.logging.level.or(acc.logging.level),
            },
            ui: Ui {
                theme: merge_theme(acc.ui.theme, layer.ui.theme),
            },
        })
    })
}

fn read_layer(path: &Path) -> Result<String> {
    fs::read_to_string(path).with_context(|| format!("failed to read config {}", path.display()))
}

fn merge_theme(base: Theme, layer: Theme) -> Theme {
    Theme {
        font: ThemeFont {
            family: layer.font.family.or(base.font.family),
        },
        window: ThemeWindow {
            default: layer.window.default.or(base.window.default),
            edge: layer.window.edge.or(base.window.edge),
        },
        surface: ThemeSurface {
            card: SurfaceCard {
                user: Card {
                    bg: layer.surface.card.user.bg.or(base.surface.card.user.bg),
                },
                assistant: Card {
                    bg: layer.surface.card.assistant.bg.or(base.surface.card.assistant.bg),
                },
            },
            compose: layer.surface.compose.or(base.surface.compose),
            text: layer.surface.text.or(base.surface.text),
        },
        fg: ThemeFg {
            default: layer.fg.default.or(base.fg.default),
            dim: layer.fg.dim.or(base.fg.dim),
            muted: layer.fg.muted.or(base.fg.muted),
        },
        border: ThemeBorder {
            default: layer.border.default.or(base.border.default),
            soft: layer.border.soft.or(base.border.soft),
            focus: layer.border.focus.or(base.border.focus),
        },
        accent: ThemeAccent {
            default: layer.accent.default.or(base.accent.default),
            user: layer.accent.user.or(base.accent.user),
            assistant: layer.accent.assistant.or(base.accent.assistant),
        },
        state: ThemeState {
            idle: layer.state.idle.or(base.state.idle),
            stream: layer.state.stream.or(base.state.stream),
            pending: layer.state.pending.or(base.state.pending),
            awaiting: layer.state.awaiting.or(base.state.awaiting),
        },
    }
}

impl Config {
    pub fn validate(&self) -> Result<()> {
        <Self as Validate>::validate(self).map_err(|report| anyhow!("config is invalid:\n{report}"))
    }
}

fn validate_log_level(value: &String, _ctx: &()) -> garde::Result {
    const ALLOWED: &[&str] = &["trace", "debug", "info", "warn", "error"];

    if !ALLOWED.contains(&value.to_lowercase().as_str()) {
        return Err(garde::Error::new(format!("must be one of {ALLOWED:?}, got '{value}'")));
    }

    Ok(())
}

fn validate_hex_color(value: &String, _ctx: &()) -> garde::Result {
    let is_valid =
        value.starts_with('#') && matches!(value.len(), 7 | 9) && value[1..].chars().all(|c| c.is_ascii_hexdigit());

    if !is_valid {
        return Err(garde::Error::new(format!(
            "must be a hex color (#RRGGBB or #RRGGBBAA), got '{value}'"
        )));
    }

    Ok(())
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
    fn defaults_populate_every_theme_token() {
        let cfg: Config = toml::from_str(DEFAULTS).expect("defaults must parse");
        let t = &cfg.ui.theme;

        assert!(t.font.family.is_some(), "font.family");

        for (n, v) in [("window.default", &t.window.default), ("window.edge", &t.window.edge)] {
            assert!(v.is_some(), "{n}");
        }

        for (n, v) in [
            ("surface.card.user.bg", &t.surface.card.user.bg),
            ("surface.card.assistant.bg", &t.surface.card.assistant.bg),
            ("surface.compose", &t.surface.compose),
            ("surface.text", &t.surface.text),
        ] {
            assert!(v.is_some(), "{n}");
        }

        for (n, v) in [
            ("fg.default", &t.fg.default),
            ("fg.dim", &t.fg.dim),
            ("fg.muted", &t.fg.muted),
        ] {
            assert!(v.is_some(), "{n}");
        }

        for (n, v) in [
            ("border.default", &t.border.default),
            ("border.soft", &t.border.soft),
            ("border.focus", &t.border.focus),
        ] {
            assert!(v.is_some(), "{n}");
        }

        for (n, v) in [
            ("accent.default", &t.accent.default),
            ("accent.user", &t.accent.user),
            ("accent.assistant", &t.accent.assistant),
        ] {
            assert!(v.is_some(), "{n}");
        }

        for (n, v) in [
            ("state.idle", &t.state.idle),
            ("state.stream", &t.state.stream),
            ("state.pending", &t.state.pending),
            ("state.awaiting", &t.state.awaiting),
        ] {
            assert!(v.is_some(), "{n}");
        }
    }

    #[test]
    fn load_merges_cli_path_over_defaults() {
        let p = write_tmp("merge.toml", "[logging]\nlevel = \"debug\"\n");
        let cfg = load(Some(&p), None).expect("load");
        assert_eq!(cfg.logging.level.as_deref(), Some("debug"));
        fs::remove_file(&p).ok();
    }

    #[test]
    fn theme_override_preserves_untouched_tokens() {
        let p = write_tmp(
            "theme.toml",
            "[ui.theme.window]\ndefault = \"#101418\"\nedge = \"#ff00aa\"\n\n[ui.theme.border]\nfocus = \"#00ff00\"\n\n[ui.theme.surface.card.user]\nbg = \"#ff8800\"\n",
        );
        let cfg = load(Some(&p), None).expect("load");

        // Overridden.
        assert_eq!(cfg.ui.theme.window.default.as_deref(), Some("#101418"));
        assert_eq!(cfg.ui.theme.window.edge.as_deref(), Some("#ff00aa"));
        assert_eq!(cfg.ui.theme.border.focus.as_deref(), Some("#00ff00"));
        assert_eq!(cfg.ui.theme.surface.card.user.bg.as_deref(), Some("#ff8800"));

        // Untouched in the same groups still fall back to defaults.
        assert_eq!(cfg.ui.theme.border.default.as_deref(), Some("#4b5263"));
        assert_eq!(cfg.ui.theme.border.soft.as_deref(), Some("#2c333d"));
        assert_eq!(cfg.ui.theme.surface.card.assistant.bg.as_deref(), Some("#22282f"));
        assert_eq!(cfg.ui.theme.surface.compose.as_deref(), Some("#2c333d"));

        // Groups not mentioned at all still come from defaults.
        assert_eq!(cfg.ui.theme.fg.default.as_deref(), Some("#abb2bf"));
        assert_eq!(cfg.ui.theme.accent.default.as_deref(), Some("#abb2bf"));

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
    fn validate_rejects_bad_log_level() {
        let cfg = Config {
            logging: Logging {
                level: Some("verbose".into()),
            },
            ..Config::default()
        };
        let err = cfg.validate().expect_err("should error");
        assert!(err.to_string().contains("logging.level"));
    }

    #[test]
    fn validate_accepts_known_levels() {
        for lvl in ["trace", "debug", "info", "warn", "error"] {
            let cfg = Config {
                logging: Logging {
                    level: Some(lvl.into()),
                },
                ..Config::default()
            };
            cfg.validate().unwrap_or_else(|e| panic!("{lvl}: {e}"));
        }
    }

    #[test]
    fn validate_rejects_bad_hex_color_in_any_group() {
        for (name, cfg) in [
            (
                "window.default",
                Config {
                    ui: Ui {
                        theme: Theme {
                            window: ThemeWindow {
                                default: Some("not-a-color".into()),
                                ..Default::default()
                            },
                            ..Default::default()
                        },
                    },
                    ..Default::default()
                },
            ),
            (
                "surface.card.user.bg",
                Config {
                    ui: Ui {
                        theme: Theme {
                            surface: ThemeSurface {
                                card: SurfaceCard {
                                    user: Card {
                                        bg: Some("#xyz".into()),
                                    },
                                    ..Default::default()
                                },
                                ..Default::default()
                            },
                            ..Default::default()
                        },
                    },
                    ..Default::default()
                },
            ),
            (
                "accent.user",
                Config {
                    ui: Ui {
                        theme: Theme {
                            accent: ThemeAccent {
                                user: Some("#12345".into()),
                                ..Default::default()
                            },
                            ..Default::default()
                        },
                    },
                    ..Default::default()
                },
            ),
        ] {
            let err = cfg.validate().expect_err(&format!("{name} should reject"));
            assert!(
                err.to_string().contains(name),
                "error for {name} missing the field path: {err}"
            );
            assert!(
                err.to_string().contains("hex color"),
                "error for {name} missing 'hex color': {err}"
            );
        }
    }

    #[test]
    fn validate_accepts_hex_with_alpha() {
        let cfg = Config {
            ui: Ui {
                theme: Theme {
                    window: ThemeWindow {
                        default: Some("#1e2127ff".into()),
                        edge: Some("#D3B051".into()),
                    },
                    ..Default::default()
                },
            },
            ..Default::default()
        };
        cfg.validate().expect("6- and 8-digit hex both accepted");
    }
}
