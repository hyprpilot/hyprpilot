//! Autostart reconcile — bring the platform's autostart entry into
//! lockstep with `[autostart] enabled` from config. Source of truth
//! is the config file; daemon edits the OS-side entry on every boot
//! to match.
//!
//! `tauri-plugin-autostart` v2 owns the per-platform mechanism:
//! - **Linux DE** — `~/.config/autostart/hyprpilot.desktop` (XDG).
//! - **macOS** — launchd plist via `LaunchAgent` mode.
//! - **Windows** — `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
//!
//! Hyprland users don't get reliable autostart from the XDG path;
//! that's what the (deferred) systemd user unit shipped with the AUR
//! package handles. The plugin remains the cross-platform fallback.

use std::sync::{Arc, RwLock};

use anyhow::Result;
use tauri::AppHandle;
use tauri_plugin_autostart::ManagerExt;
use tracing::{info, warn};

use crate::config::Config;

/// Read `[autostart] enabled` and call the plugin's enable / disable
/// API to match. Logs the action taken (or `no-op`) at `info!` level;
/// failures degrade to `warn!` + continue so an autostart misconfig
/// (e.g. write-protected `~/.config/autostart`) never aborts the
/// daemon's boot.
pub fn reconcile(app: &AppHandle, shared_config: &Arc<RwLock<Config>>) -> Result<()> {
    let want = shared_config
        .read()
        .expect("config rwlock poisoned")
        .autostart
        .enabled
        .unwrap_or(false);

    let manager = app.autolaunch();

    let have = match manager.is_enabled() {
        Ok(v) => v,
        Err(err) => {
            warn!(%err, "autostart: is_enabled() failed — skipping reconcile");
            return Ok(());
        }
    };

    match (want, have) {
        (true, false) => match manager.enable() {
            Ok(()) => info!("autostart: enabled (config = true, was disabled)"),
            Err(err) => warn!(%err, "autostart: enable() failed"),
        },
        (false, true) => match manager.disable() {
            Ok(()) => info!("autostart: disabled (config = false, was enabled)"),
            Err(err) => warn!(%err, "autostart: disable() failed"),
        },
        (true, true) => info!("autostart: already enabled — no-op"),
        (false, false) => info!("autostart: already disabled — no-op"),
    }

    Ok(())
}
