//! Theme palette tokens. Each leaf is `Option<HexColor>` (or
//! `Option<String>` for font stacks) so partial overrides in user TOML
//! compose cleanly over the compiled defaults layer. `HexColor` is a
//! validating newtype; the wire shape is a bare string.

use garde::Validate;
use merge::Merge;
use serde::{Deserialize, Serialize};

use super::merge_strategies::overwrite_some;

/// `#RRGGBB` or `#RRGGBBAA`. `#[serde(transparent)]` keeps the wire
/// shape a bare string; `impl Validate` runs under `#[garde(dive)]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct HexColor(pub String);

impl Validate for HexColor {
    type Context = ();

    fn validate_into(&self, _ctx: &Self::Context, parent: &mut dyn FnMut() -> garde::Path, report: &mut garde::Report) {
        let v = &self.0;
        let ok = v.starts_with('#') && matches!(v.len(), 7 | 9) && v[1..].chars().all(|c| c.is_ascii_hexdigit());
        if !ok {
            report.append(
                parent(),
                garde::Error::new(format!("must be a hex color (#RRGGBB or #RRGGBBAA), got '{v}'")),
            );
        }
    }
}

impl AsRef<str> for HexColor {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl std::ops::Deref for HexColor {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

impl From<&str> for HexColor {
    fn from(s: &str) -> Self {
        Self(s.to_owned())
    }
}

impl From<String> for HexColor {
    fn from(s: String) -> Self {
        Self(s)
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
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
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
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
    #[garde(dive)]
    pub kind: ThemeKind,
    #[garde(dive)]
    pub status: ThemeStatus,
    #[garde(dive)]
    pub permission: ThemePermission,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields)]
#[merge(strategy = overwrite_some)]
pub struct ThemeFont {
    #[garde(inner(length(min = 1)))]
    pub mono: Option<String>,
    #[garde(inner(length(min = 1)))]
    pub sans: Option<String>,
}

/// Window frame tokens. `default` = background fill; `edge` = accent stripe.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields)]
#[merge(strategy = overwrite_some)]
pub struct ThemeWindow {
    #[garde(dive)]
    pub default: Option<HexColor>,
    #[garde(dive)]
    pub edge: Option<HexColor>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeSurface {
    #[garde(dive)]
    #[merge(strategy = overwrite_some)]
    pub default: Option<HexColor>,
    #[garde(dive)]
    #[merge(strategy = overwrite_some)]
    pub bg: Option<HexColor>,
    #[garde(dive)]
    #[merge(strategy = overwrite_some)]
    pub alt: Option<HexColor>,
    #[garde(dive)]
    pub card: SurfaceCard,
    #[garde(dive)]
    #[merge(strategy = overwrite_some)]
    pub compose: Option<HexColor>,
    #[garde(dive)]
    #[merge(strategy = overwrite_some)]
    pub text: Option<HexColor>,
}

/// Message cards, keyed by speaker.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields)]
pub struct SurfaceCard {
    #[garde(dive)]
    pub user: Card,
    #[garde(dive)]
    pub assistant: Card,
}

/// One card's tokens. `bg` today; future accent/border/fg slot in alongside.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields)]
#[merge(strategy = overwrite_some)]
pub struct Card {
    #[garde(dive)]
    pub bg: Option<HexColor>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields)]
#[merge(strategy = overwrite_some)]
pub struct ThemeFg {
    #[garde(dive)]
    pub default: Option<HexColor>,
    #[garde(dive)]
    pub ink_2: Option<HexColor>,
    #[garde(dive)]
    pub dim: Option<HexColor>,
    #[garde(dive)]
    pub faint: Option<HexColor>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields)]
#[merge(strategy = overwrite_some)]
pub struct ThemeBorder {
    #[garde(dive)]
    pub default: Option<HexColor>,
    #[garde(dive)]
    pub soft: Option<HexColor>,
    #[garde(dive)]
    pub focus: Option<HexColor>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields)]
#[merge(strategy = overwrite_some)]
pub struct ThemeAccent {
    #[garde(dive)]
    pub default: Option<HexColor>,
    #[garde(dive)]
    pub user: Option<HexColor>,
    #[garde(dive)]
    pub user_soft: Option<HexColor>,
    #[garde(dive)]
    pub assistant: Option<HexColor>,
    #[garde(dive)]
    pub assistant_soft: Option<HexColor>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields)]
#[merge(strategy = overwrite_some)]
pub struct ThemeState {
    #[garde(dive)]
    pub idle: Option<HexColor>,
    #[garde(dive)]
    pub stream: Option<HexColor>,
    #[garde(dive)]
    pub pending: Option<HexColor>,
    #[garde(dive)]
    pub awaiting: Option<HexColor>,
    #[garde(dive)]
    pub working: Option<HexColor>,
}

/// Per-tool-family dispatch colours keyed by `ToolCall.kind`.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields)]
#[merge(strategy = overwrite_some)]
pub struct ThemeKind {
    #[garde(dive)]
    pub read: Option<HexColor>,
    #[garde(dive)]
    pub write: Option<HexColor>,
    #[garde(dive)]
    pub bash: Option<HexColor>,
    #[garde(dive)]
    pub search: Option<HexColor>,
    #[garde(dive)]
    pub agent: Option<HexColor>,
    #[garde(dive)]
    pub think: Option<HexColor>,
    #[garde(dive)]
    pub terminal: Option<HexColor>,
    #[garde(dive)]
    pub acp: Option<HexColor>,
}

/// Toast / banner status hues. Distinct from the `state` machine —
/// `ok`/`warn`/`err` are one-shot notifications, not phase transitions.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields)]
#[merge(strategy = overwrite_some)]
pub struct ThemeStatus {
    #[garde(dive)]
    pub ok: Option<HexColor>,
    #[garde(dive)]
    pub warn: Option<HexColor>,
    #[garde(dive)]
    pub err: Option<HexColor>,
}

/// Warm-brown panel fills for the permission stack.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields)]
#[merge(strategy = overwrite_some)]
pub struct ThemePermission {
    #[garde(dive)]
    pub bg: Option<HexColor>,
    #[garde(dive)]
    pub bg_active: Option<HexColor>,
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write;
    use std::path::PathBuf;

    use super::super::{load, Config, DEFAULTS};
    use super::*;

    fn write_tmp(name: &str, body: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("hyprpilot-test-{}-{}", std::process::id(), name));
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(body.as_bytes()).unwrap();

        path
    }

    #[test]
    fn defaults_populate_every_theme_token() {
        let cfg: Config = toml::from_str(DEFAULTS).expect("defaults must parse");
        let t = &cfg.ui.theme;

        assert!(t.font.mono.is_some(), "font.mono");
        assert!(t.font.sans.is_some(), "font.sans");

        for (n, v) in [("window.default", &t.window.default), ("window.edge", &t.window.edge)] {
            assert!(v.is_some(), "{n}");
        }

        for (n, v) in [
            ("surface.default", &t.surface.default),
            ("surface.bg", &t.surface.bg),
            ("surface.alt", &t.surface.alt),
            ("surface.card.user.bg", &t.surface.card.user.bg),
            ("surface.card.assistant.bg", &t.surface.card.assistant.bg),
            ("surface.compose", &t.surface.compose),
            ("surface.text", &t.surface.text),
        ] {
            assert!(v.is_some(), "{n}");
        }

        for (n, v) in [
            ("fg.default", &t.fg.default),
            ("fg.ink_2", &t.fg.ink_2),
            ("fg.dim", &t.fg.dim),
            ("fg.faint", &t.fg.faint),
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
            ("accent.user_soft", &t.accent.user_soft),
            ("accent.assistant", &t.accent.assistant),
            ("accent.assistant_soft", &t.accent.assistant_soft),
        ] {
            assert!(v.is_some(), "{n}");
        }

        for (n, v) in [
            ("state.idle", &t.state.idle),
            ("state.stream", &t.state.stream),
            ("state.pending", &t.state.pending),
            ("state.awaiting", &t.state.awaiting),
            ("state.working", &t.state.working),
        ] {
            assert!(v.is_some(), "{n}");
        }

        for (n, v) in [
            ("kind.read", &t.kind.read),
            ("kind.write", &t.kind.write),
            ("kind.bash", &t.kind.bash),
            ("kind.search", &t.kind.search),
            ("kind.agent", &t.kind.agent),
            ("kind.think", &t.kind.think),
            ("kind.terminal", &t.kind.terminal),
            ("kind.acp", &t.kind.acp),
        ] {
            assert!(v.is_some(), "{n}");
        }

        for (n, v) in [
            ("status.ok", &t.status.ok),
            ("status.warn", &t.status.warn),
            ("status.err", &t.status.err),
        ] {
            assert!(v.is_some(), "{n}");
        }

        for (n, v) in [
            ("permission.bg", &t.permission.bg),
            ("permission.bg_active", &t.permission.bg_active),
        ] {
            assert!(v.is_some(), "{n}");
        }
    }

    #[test]
    fn theme_override_preserves_untouched_tokens() {
        let p = write_tmp(
            "theme.toml",
            r##"
[ui.theme.window]
default = "#101418"
edge = "#ff00aa"

[ui.theme.border]
focus = "#00ff00"

[ui.theme.surface.card.user]
bg = "#ff8800"

[ui.theme.kind]
read = "#123456"
"##,
        );
        let cfg = load(Some(&p), None).expect("load");

        // Overridden.
        assert_eq!(cfg.ui.theme.window.default.as_deref(), Some("#101418"));
        assert_eq!(cfg.ui.theme.window.edge.as_deref(), Some("#ff00aa"));
        assert_eq!(cfg.ui.theme.border.focus.as_deref(), Some("#00ff00"));
        assert_eq!(cfg.ui.theme.surface.card.user.bg.as_deref(), Some("#ff8800"));
        assert_eq!(cfg.ui.theme.kind.read.as_deref(), Some("#123456"));

        // Untouched in the same groups still fall back to defaults.
        assert_eq!(cfg.ui.theme.border.default.as_deref(), Some("#20242e"));
        assert_eq!(cfg.ui.theme.border.soft.as_deref(), Some("#2b2f3b"));
        assert_eq!(cfg.ui.theme.surface.card.assistant.bg.as_deref(), Some("#12141a"));
        assert_eq!(cfg.ui.theme.surface.compose.as_deref(), Some("#181b22"));
        assert_eq!(cfg.ui.theme.kind.write.as_deref(), Some("#e480d4"));

        // Groups not mentioned at all still come from defaults.
        assert_eq!(cfg.ui.theme.fg.default.as_deref(), Some("#d8dde5"));
        assert_eq!(cfg.ui.theme.accent.default.as_deref(), Some("#c99bf0"));
        assert_eq!(cfg.ui.theme.status.ok.as_deref(), Some("#7fcf8a"));
        assert_eq!(cfg.ui.theme.permission.bg.as_deref(), Some("#18130a"));

        fs::remove_file(&p).ok();
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
