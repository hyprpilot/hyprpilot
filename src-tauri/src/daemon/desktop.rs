//! Desktop integration — signals the overlay reads from the user's
//! environment that aren't part of `[ui]` config.
//!
//! Today: just `$HOME` for webview-side cwd display. Earlier rounds
//! probed the user's GTK font; that path was deleted in favor of the
//! CSS `ui-sans-serif` / `system-ui` keywords (the webview already
//! resolves them to the platform default), so the daemon no longer
//! reaches for the desktop font setting.
//!
//! Future XDG paths, monitor scale overrides, and other
//! desktop-environment knobs land here when they earn their slot.

/// Resolved home directory for the running user. The webview cannot
/// read it itself (no `process` global, no `~` expansion in the
/// renderer), so the daemon hands it off via Tauri command.
///
/// Goes through `paths::home_dir` (cached `BaseDirs`) so the mapping
/// is correct on every platform we ship to.
#[tauri::command]
pub(crate) fn get_home_dir() -> String {
    crate::paths::home_dir().to_string_lossy().into_owned()
}
