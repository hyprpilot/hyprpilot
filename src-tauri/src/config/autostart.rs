//! `[autostart]` config tree. Top-level — autostart is a property of
//! the binary's relationship to the OS, not of the daemon's internal
//! config. Drives the boot-time reconcile against
//! `tauri-plugin-autostart`'s per-platform mechanism (XDG `.desktop`
//! on Linux DEs, launchd plist on macOS, registry on Windows).
//!
//! The captain edits this file; daemon reads on boot and calls
//! `Manager::enable()` / `Manager::disable()` to match. Edit-and-
//! restart-the-daemon is the loop. No imperative `ctl autostart`
//! subcommand today — config is the source.

use garde::Validate;
use merge::Merge;
use serde::{Deserialize, Serialize};

use super::merge_strategies::overwrite_some;

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Validate, Merge)]
#[serde(default, deny_unknown_fields)]
#[merge(strategy = overwrite_some)]
pub struct Autostart {
    /// `true` — daemon launches at user login through
    /// `tauri-plugin-autostart`. `false` (default) — captain runs
    /// the daemon manually or wires `exec-once = hyprpilot` in
    /// their compositor config. Hyprland users on AUR will land on
    /// the systemd user unit shipped with the package; this flag
    /// remains the cross-platform fallback.
    #[garde(skip)]
    pub enabled: Option<bool>,
}
