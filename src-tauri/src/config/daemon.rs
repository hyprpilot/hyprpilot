//! `[daemon]` config tree. Owns the window-surface knobs (anchor / center
//! mode + per-mode geometry) consumed by `WindowRenderer`.

use std::path::PathBuf;

use garde::Validate;
use merge::Merge;
use serde::{Deserialize, Serialize};

use super::merge_strategies::overwrite_some;

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields)]
pub struct Daemon {
    #[garde(skip)]
    #[merge(strategy = overwrite_some)]
    pub socket: Option<PathBuf>,
    #[garde(dive)]
    pub window: Window,
}

/// `[daemon.window]`. See CLAUDE.md "Window surface" for why `layer`
/// and `keyboard_interactivity` aren't config knobs.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields)]
pub struct Window {
    #[garde(skip)]
    #[merge(strategy = overwrite_some)]
    pub mode: Option<WindowMode>,
    #[garde(inner(length(min = 1)))]
    #[merge(strategy = overwrite_some)]
    pub output: Option<String>,
    #[garde(dive)]
    pub anchor: AnchorWindow,
    #[garde(dive)]
    pub center: CenterWindow,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum WindowMode {
    #[default]
    Anchor,
    Center,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Edge {
    Top,
    #[default]
    Right,
    Bottom,
    Left,
}

/// Anchor-mode geometry. Unset `height` → full-height (top+bottom+edge pin).
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields)]
#[merge(strategy = overwrite_some)]
pub struct AnchorWindow {
    #[garde(skip)]
    pub edge: Option<Edge>,
    #[garde(inner(range(min = 0, max = 10_000)))]
    pub margin: Option<i32>,
    #[garde(dive)]
    pub width: Option<Dimension>,
    #[garde(dive)]
    pub height: Option<Dimension>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields)]
#[merge(strategy = overwrite_some)]
pub struct CenterWindow {
    #[garde(dive)]
    pub width: Option<Dimension>,
    #[garde(dive)]
    pub height: Option<Dimension>,
}

/// Pixel literal or a `"N%"` string. TOML accepts either at the same
/// key; custom `Deserialize` handles the `%` suffix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(untagged)]
pub enum Dimension {
    Pixels(u32),
    Percent(u8),
}

impl Validate for Dimension {
    type Context = ();

    fn validate_into(&self, _ctx: &Self::Context, parent: &mut dyn FnMut() -> garde::Path, report: &mut garde::Report) {
        match *self {
            Dimension::Pixels(0) => {
                report.append(parent(), garde::Error::new("pixel dimension must be >= 1"));
            }
            Dimension::Pixels(px) if px > 10_000 => {
                report.append(
                    parent(),
                    garde::Error::new(format!("pixel dimension {px} exceeds 10000 — refusing absurd size")),
                );
            }
            Dimension::Pixels(_) => {}
            Dimension::Percent(p) if (1..=100).contains(&p) => {}
            Dimension::Percent(p) => {
                report.append(parent(), garde::Error::new(format!("percent must be 1..=100, got {p}")));
            }
        }
    }
}

impl<'de> Deserialize<'de> for Dimension {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        use serde::de::Error as _;

        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Int(i64),
            Str(String),
        }

        match Raw::deserialize(deserializer)? {
            Raw::Int(n) => {
                let px: u32 = n.try_into().map_err(|_| {
                    D::Error::custom(format!("pixel dimension must be a non-negative integer, got {n}"))
                })?;

                Ok(Dimension::Pixels(px))
            }
            Raw::Str(s) => {
                let trimmed = s.trim();

                let digits = trimmed
                    .strip_suffix('%')
                    .ok_or_else(|| D::Error::custom(format!("dimension string must end with '%', got {s:?}")))?;

                let n: u8 = digits
                    .parse()
                    .map_err(|e| D::Error::custom(format!("invalid percent value {digits:?}: {e}")))?;

                Ok(Dimension::Percent(n))
            }
        }
    }
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

    /// The daemon consumes several `Option<T>` window fields via `.expect()`
    /// rather than carrying a second-layer Rust default — defaults.toml is
    /// the single source of truth. If a field is removed from the TOML
    /// without removing the `.expect()` call, the daemon panics at startup;
    /// this test fails before we ship that.
    #[test]
    fn defaults_populate_every_daemon_window_field() {
        let cfg: Config = toml::from_str(DEFAULTS).expect("defaults must parse");
        let w = &cfg.daemon.window;

        assert!(w.mode.is_some(), "daemon.window.mode");
        assert!(w.anchor.edge.is_some(), "daemon.window.anchor.edge");
        assert!(w.anchor.margin.is_some(), "daemon.window.anchor.margin");
        assert!(w.anchor.width.is_some(), "daemon.window.anchor.width");
        // anchor.height intentionally optional — None means full-height fill.
        assert!(w.center.width.is_some(), "daemon.window.center.width");
        assert!(w.center.height.is_some(), "daemon.window.center.height");
    }

    #[test]
    fn daemon_window_override_preserves_untouched_tokens() {
        let p = write_tmp(
            "window.toml",
            r#"
[daemon.window]
mode = "center"

[daemon.window.anchor]
edge = "left"

[daemon.window.center]
width = "70%"
"#,
        );
        let cfg = load(Some(&p), None).expect("load");

        // Overridden fields.
        assert_eq!(cfg.daemon.window.mode, Some(WindowMode::Center));
        assert_eq!(cfg.daemon.window.anchor.edge, Some(Edge::Left));
        assert_eq!(cfg.daemon.window.center.width, Some(Dimension::Percent(70)));

        // Untouched within the same subtree — fall through to defaults.
        assert_eq!(cfg.daemon.window.anchor.margin, Some(0));
        assert_eq!(cfg.daemon.window.anchor.width, Some(Dimension::Percent(40)));
        // Height is intentionally unset in defaults — signals full-height fill.
        assert_eq!(cfg.daemon.window.anchor.height, None);
        assert_eq!(cfg.daemon.window.center.height, Some(Dimension::Percent(50)));

        fs::remove_file(&p).ok();
    }

    #[test]
    fn dimension_parses_pixels_and_percent() {
        #[derive(Debug, Deserialize)]
        struct Holder {
            d: Dimension,
        }

        let pixels: Holder = toml::from_str("d = 480").unwrap();
        assert_eq!(pixels.d, Dimension::Pixels(480));

        let percent: Holder = toml::from_str("d = \"50%\"").unwrap();
        assert_eq!(percent.d, Dimension::Percent(50));

        // Non-percent string shape — rejected at parse time.
        let err = toml::from_str::<Holder>("d = \"50px\"").expect_err("should reject");
        assert!(err.to_string().contains("must end with '%'"), "{err}");

        // Interior whitespace between digits and '%' must reject — the `.trim()`
        // between `strip_suffix('%')` and `parse()` used to silently accept
        // `"50 %"`. Outer whitespace is still fine (serde/the surrounding
        // `trim()` handles it).
        let err2 = toml::from_str::<Holder>("d = \"50 %\"").expect_err("should reject interior whitespace");
        assert!(err2.to_string().contains("invalid percent"), "{err2}");
    }

    #[test]
    fn validate_rejects_oversized_percent_dimension() {
        let p = write_tmp(
            "bad-pct.toml",
            r#"
[daemon.window.center]
width = "200%"
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        let err = cfg.validate().expect_err("should reject");
        let msg = err.to_string();
        assert!(msg.contains("daemon.window.center.width"), "{msg}");
        assert!(msg.contains("1..=100"), "{msg}");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn validate_rejects_negative_anchor_margin() {
        let p = write_tmp(
            "bad-margin.toml",
            r#"
[daemon.window.anchor]
margin = -5
"#,
        );
        let cfg = load(Some(&p), None).expect("parses");
        let err = cfg.validate().expect_err("should reject");
        let msg = err.to_string();
        assert!(msg.contains("daemon.window.anchor.margin"), "{msg}");
        fs::remove_file(&p).ok();
    }

    #[test]
    fn validate_rejects_zero_pixel_dimension() {
        let cfg = Config {
            daemon: Daemon {
                window: Window {
                    center: CenterWindow {
                        width: Some(Dimension::Pixels(0)),
                        ..Default::default()
                    },
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let err = cfg.validate().expect_err("should error");
        assert!(err.to_string().contains(">= 1"), "{err}");
    }
}
