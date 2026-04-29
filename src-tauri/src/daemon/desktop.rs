//! Desktop integration — signals the overlay reads from the user's
//! environment that aren't part of `[ui]` config: GTK font (drives
//! page zoom + sans family override), `$HOME` (drives webview-side
//! `~` collapse on cwd display).
//!
//! Future XDG paths, monitor scale overrides, and other
//! desktop-environment knobs land here too.

use serde::Serialize;
use tauri::State;

/// User-desktop GTK font setting, parsed from `gtk-font-name` on the
/// default `gtk::Settings`. Stored in managed state at boot so the
/// webview can read the base font size synchronously. `None` when the
/// GTK query fails (no settings singleton or unparseable font string)
/// — the CSS fallback (browser default) is the correct behaviour
/// then.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GtkFont {
    pub family: String,
    pub size_pt: f32,
}

#[tauri::command]
pub(crate) fn get_gtk_font(font: State<'_, Option<GtkFont>>) -> Option<GtkFont> {
    font.inner().clone()
}

/// Resolved home directory for the running user. The webview cannot
/// read it itself (no `process` global, no `~` expansion in the
/// renderer), so the daemon hands it off via Tauri command. `None`
/// when no home directory is resolvable — the consumer (cwd
/// truncation) skips the `~` collapse step then.
///
/// Goes through the `home` crate so the mapping is correct on every
/// platform we ship to (Linux + macOS today, Windows-safe at zero
/// cost should that ever land).
#[tauri::command]
pub(crate) fn get_home_dir() -> Option<String> {
    home::home_dir().map(|p| p.to_string_lossy().into_owned())
}

/// Parse a GTK font string ("Inter 10", "JetBrains Mono Bold 11",
/// "Sans 10") into `{ family, size_pt }`. The last whitespace-
/// separated token is the point size; every preceding token is family
/// (joined back with spaces). Returns `None` when the trailing token
/// isn't a valid positive float or the input is missing a size.
fn parse_gtk_font(raw: &str) -> Option<GtkFont> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let (family, size) = trimmed.rsplit_once(char::is_whitespace)?;
    let size_pt: f32 = size.parse().ok()?;
    if !(size_pt.is_finite() && size_pt > 0.0) {
        return None;
    }
    let family = family.trim();
    if family.is_empty() {
        return None;
    }
    Some(GtkFont {
        family: family.to_string(),
        size_pt,
    })
}

/// Query the active GTK `gtk-font-name` setting. Must run on the GTK
/// main thread (the setup closure is, since Tauri has already called
/// `gtk::init` by then).
#[cfg(target_os = "linux")]
pub(crate) fn query_gtk_font() -> Option<GtkFont> {
    use gtk::prelude::GtkSettingsExt;
    let Some(settings) = gtk::Settings::default() else {
        tracing::warn!("gtk::Settings::default() returned None; base font will fall back to browser default");
        return None;
    };
    let Some(name) = settings.gtk_font_name() else {
        tracing::warn!("gtk-font-name is unset; base font will fall back to browser default");
        return None;
    };
    let raw = name.as_str();
    match parse_gtk_font(raw) {
        Some(font) => {
            tracing::info!(raw = raw, family = %font.family, size_pt = font.size_pt, "parsed GTK font");
            Some(font)
        }
        None => {
            tracing::warn!(
                raw = raw,
                "failed to parse GTK font name; expected form `<family> <size_pt>`"
            );
            None
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn query_gtk_font() -> Option<GtkFont> {
    None
}

#[cfg(test)]
mod tests {
    use super::parse_gtk_font;

    #[test]
    fn parses_simple_family_and_size() {
        let f = parse_gtk_font("Inter 10").unwrap();
        assert_eq!(f.family, "Inter");
        assert_eq!(f.size_pt, 10.0);
    }

    #[test]
    fn parses_multi_word_family() {
        let f = parse_gtk_font("JetBrains Mono Bold 11").unwrap();
        assert_eq!(f.family, "JetBrains Mono Bold");
        assert_eq!(f.size_pt, 11.0);
    }

    #[test]
    fn parses_fractional_size() {
        let f = parse_gtk_font("Sans 10.5").unwrap();
        assert_eq!(f.family, "Sans");
        assert_eq!(f.size_pt, 10.5);
    }

    #[test]
    fn rejects_empty_input() {
        assert!(parse_gtk_font("").is_none());
        assert!(parse_gtk_font("   ").is_none());
    }

    #[test]
    fn rejects_missing_size() {
        assert!(parse_gtk_font("Inter").is_none());
    }

    #[test]
    fn rejects_non_numeric_trailing_token() {
        assert!(parse_gtk_font("Inter regular").is_none());
    }

    #[test]
    fn rejects_non_positive_size() {
        assert!(parse_gtk_font("Inter 0").is_none());
        assert!(parse_gtk_font("Inter -5").is_none());
    }
}
