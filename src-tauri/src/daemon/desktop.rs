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

/// The daemon's own working directory at the time of the call. Drives
/// the idle-banner context line so the captain sees where new
/// instances will land before they spawn one. Reflects whatever
/// `hyprpilot daemon --cwd <DIR>` was passed (or the spawning shell's
/// cwd when the flag was omitted). Falls back to `/` on
/// `current_dir()` failure — the only realistic source is the daemon's
/// cwd having been deleted out from under it, in which case
/// "unknown" reads cleaner than a noisy error.
#[tauri::command]
pub(crate) fn get_daemon_cwd() -> String {
    std::env::current_dir()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "/".to_string())
}

/// Maximum bytes shipped to the webview as a file attachment body.
/// Same cap as the path-completion preview — if a captain attaches a
/// 1GB log file they get the head 256KB and the rest stays on disk.
const FILE_ATTACHMENT_MAX_BYTES: usize = 256 * 1024;

/// Read a file's contents for use as an `@./path` attachment payload.
/// Captain's path-completion commit in the composer pipes through here;
/// the file's text body becomes the attachment's `body`. Binary files
/// resolve to a stub describing the size + path so the agent gets a
/// pointer instead of mojibake. `~` and env-var expansion mirrors the
/// rest of the path surfaces.
#[tauri::command]
pub(crate) async fn read_file_for_attachment(path: String) -> Result<serde_json::Value, String> {
    let expanded = shellexpand::full(&path)
        .map(|s| s.into_owned())
        .unwrap_or_else(|_| path.clone());
    let metadata = tokio::fs::metadata(&expanded)
        .await
        .map_err(|err| format!("read_file_for_attachment: stat {expanded}: {err}"))?;
    if metadata.is_dir() {
        return Err(format!("'{expanded}' is a directory; attach a file"));
    }
    let bytes = tokio::fs::read(&expanded)
        .await
        .map_err(|err| format!("read_file_for_attachment: read {expanded}: {err}"))?;
    let truncated = bytes.len() > FILE_ATTACHMENT_MAX_BYTES;
    let head = if truncated {
        &bytes[..FILE_ATTACHMENT_MAX_BYTES]
    } else {
        &bytes[..]
    };

    if head.contains(&0) {
        return Ok(serde_json::json!({
            "path": expanded,
            "body": format!("[binary file — {} bytes]", metadata.len()),
            "binary": true,
            "truncated": false,
        }));
    }
    let body = String::from_utf8_lossy(head).into_owned();
    Ok(serde_json::json!({
        "path": expanded,
        "body": body,
        "binary": false,
        "truncated": truncated,
    }))
}
