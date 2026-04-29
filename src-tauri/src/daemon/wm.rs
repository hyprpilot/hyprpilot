//! Adapter over the user's compositor / window manager. Supplies the
//! session-level information the overlay needs that isn't cleanly
//! exposed through Tauri or GDK directly — today that's "which monitor
//! is the user focused on". Tomorrow it may grow to cover focused
//! workspace, focus-follows-mouse state, or active-window geometry as
//! K-239 and later issues add richer context.
//!
//! One implementation per WM family. `detect()` picks the right one at
//! runtime via environment markers:
//! - `HYPRLAND_INSTANCE_SIGNATURE` → `WindowManagerHyprland`
//!   (queries `hyprctl -j monitors`)
//! - `SWAYSOCK` → `WindowManagerSway`
//!   (queries `swaymsg -t get_outputs`)
//! - otherwise → `WindowManagerGtk` — cursor-position via GDK's seat
//!   pointer; works on X11 and on any Wayland compositor whose GDK
//!   backend populates pointer state. Used as the fallback when we
//!   don't have a known compositor IPC socket to query.

use std::process::Command;
use std::sync::Arc;

use serde_json::Value;
use tauri::Monitor;
use tracing::debug;

/// Canonical identity for a physical monitor / output as the WM
/// reports it. The connector `name` (e.g. `DP-1`, `eDP-1`) is the
/// authoritative key — same namespace Tauri's `Monitor::name()` uses
/// and what `[daemon.window] output` is matched against. `make`,
/// `model`, and `serial` come from the monitor's EDID (populated
/// from Hyprland / Sway IPC) and are metadata: useful for log lines
/// and for stricter disambiguation later (e.g. two identical
/// monitors connected to the same port across reboots), not load-
/// bearing today.
///
/// All metadata fields are `Option` because not every source
/// populates them — GDK exposes `manufacturer` / `model` but not
/// `serial`, virtual outputs carry no EDID, and the GTK fallback
/// path currently only resolves `name` reliably.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorInfo {
    pub name: String,
    pub make: Option<String>,
    pub model: Option<String>,
    pub serial: Option<String>,
}

pub trait WindowManager: Send + Sync {
    /// Identifier for log lines — makes it obvious which adapter
    /// answered a given `focused_monitor` call without inspecting the
    /// type name of an `Arc<dyn ...>`.
    fn name(&self) -> &'static str;

    /// Return the monitor the user is currently focused on. `monitors`
    /// is Tauri's view of the monitor set — used by the GTK fallback
    /// to bounds-check the cursor position against real outputs, and
    /// ignored by compositor-IPC-backed impls that get the focused
    /// output name directly from the wire. Returns `None` when focus
    /// state is unavailable — the caller falls through to its own
    /// fallbacks (primary → any).
    fn focused_monitor(&self, monitors: &[Monitor]) -> Option<MonitorInfo>;
}

/// Pick the `WindowManager` implementation for the current session.
/// Hyprland wins when both `HYPRLAND_INSTANCE_SIGNATURE` and
/// `SWAYSOCK` are set (shouldn't happen in practice), since the
/// Hyprland IPC is more featureful.
pub fn detect() -> Arc<dyn WindowManager> {
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() {
        debug!("wm: detected Hyprland");
        Arc::new(WindowManagerHyprland)
    } else if std::env::var_os("SWAYSOCK").is_some() {
        debug!("wm: detected Sway");
        Arc::new(WindowManagerSway)
    } else {
        debug!("wm: no compositor IPC detected — using GTK/GDK fallback");
        Arc::new(WindowManagerGtk)
    }
}

// ---------------------------------------------------------------------------
// Hyprland — `hyprctl -j monitors` returns an array; the focused monitor
// carries `"focused": true` and a `"name"` connector string (matches
// Tauri's `Monitor::name()`).
// ---------------------------------------------------------------------------

pub struct WindowManagerHyprland;

impl WindowManager for WindowManagerHyprland {
    fn name(&self) -> &'static str {
        "hyprland"
    }

    fn focused_monitor(&self, _monitors: &[Monitor]) -> Option<MonitorInfo> {
        let json = ipc_json("hyprctl", &["-j", "monitors"])?;
        let info = focused_monitor_info(&json, /* focused_key */ "focused")?;
        debug!(
            name = %info.name,
            make = ?info.make,
            model = ?info.model,
            serial = ?info.serial,
            "hyprland: focused monitor",
        );
        Some(info)
    }
}

// ---------------------------------------------------------------------------
// Sway — `swaymsg -t get_outputs` returns an array with the same shape;
// the focused output has `"focused": true` and a `"name"` field.
// ---------------------------------------------------------------------------

pub struct WindowManagerSway;

impl WindowManager for WindowManagerSway {
    fn name(&self) -> &'static str {
        "sway"
    }

    fn focused_monitor(&self, _monitors: &[Monitor]) -> Option<MonitorInfo> {
        let json = ipc_json("swaymsg", &["-t", "get_outputs", "-r"])?;
        let info = focused_monitor_info(&json, "focused")?;
        debug!(
            name = %info.name,
            make = ?info.make,
            model = ?info.model,
            serial = ?info.serial,
            "sway: focused monitor",
        );
        Some(info)
    }
}

// ---------------------------------------------------------------------------
// GTK / GDK fallback — asks GDK for the current pointer position and
// returns the monitor whose bounds contain it. Works on X11 always; on
// Wayland it depends on whether the compositor's GDK backend populates
// pointer state (often stale as `(0, 0)` on multi-monitor Wayland
// setups, which is why the specific compositor adapters exist).
// ---------------------------------------------------------------------------

pub struct WindowManagerGtk;

impl WindowManager for WindowManagerGtk {
    fn name(&self) -> &'static str {
        "gtk"
    }

    fn focused_monitor(&self, monitors: &[Monitor]) -> Option<MonitorInfo> {
        let (x, y) = cursor_position_gdk()?;
        debug!(x, y, "gtk: pointer position");
        let tauri_mon = monitors.iter().find(|m| {
            let pos = m.position();
            let size = m.size();
            x >= pos.x && x < pos.x + size.width as i32 && y >= pos.y && y < pos.y + size.height as i32
        })?;
        let name = tauri_mon.name().cloned()?;
        // `make` / `model` are available via `gdk::Monitor::manufacturer()` /
        // `.model()` but matching a `tauri::Monitor` back to a `gdk::Monitor`
        // needs the geometry hop implemented in `renderer::gdk_monitor_for`.
        // Skip it here; the name alone is enough for the identity key, and
        // this path is already a fallback for sessions without a WM IPC
        // socket. If someone needs make/model enrichment on the GTK path
        // later, lift `gdk_monitor_for` to `wm.rs` and pull from it.
        let info = MonitorInfo {
            name,
            make: None,
            model: None,
            serial: None,
        };
        debug!(name = %info.name, "gtk: focused monitor (name only — EDID metadata unavailable without gdk hop)");
        Some(info)
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Shell out to a compositor IPC CLI, parse stdout as JSON. Returns
/// `None` on spawn failure, non-zero exit, or JSON parse error. Every
/// failure path logs at `debug!` so `RUST_LOG=hyprpilot=debug` surfaces
/// the reason without noise in normal runs.
fn ipc_json(bin: &str, args: &[&str]) -> Option<Value> {
    let out = match Command::new(bin).args(args).output() {
        Ok(o) => o,
        Err(err) => {
            debug!(%bin, %err, "wm ipc: spawn failed");
            return None;
        }
    };
    if !out.status.success() {
        debug!(%bin, status = ?out.status, "wm ipc: exited non-zero");
        return None;
    }
    match serde_json::from_slice(&out.stdout) {
        Ok(v) => Some(v),
        Err(err) => {
            debug!(%bin, %err, "wm ipc: stdout is not valid JSON");
            None
        }
    }
}

/// Both `hyprctl -j monitors` and `swaymsg -t get_outputs -r` emit
/// `[{ "focused": bool, "name": "...", "make": "...", "model": "...",
/// "serial": "...", ... }, ...]` with matching field names. Pull the
/// first entry where `focused == true` and project it into our
/// canonical `MonitorInfo` identity.
fn focused_monitor_info(json: &Value, focused_key: &str) -> Option<MonitorInfo> {
    let focused = json
        .as_array()?
        .iter()
        .find(|m| m.get(focused_key).and_then(Value::as_bool) == Some(true))?;
    let string_at = |key: &str| focused.get(key).and_then(Value::as_str).map(str::to_owned);
    Some(MonitorInfo {
        name: string_at("name")?,
        make: string_at("make"),
        model: string_at("model"),
        serial: string_at("serial"),
    })
}

#[cfg(target_os = "linux")]
fn cursor_position_gdk() -> Option<(i32, i32)> {
    use gdk::prelude::*;

    let display = gdk::Display::default()?;
    let seat = display.default_seat()?;
    let pointer = seat.pointer()?;
    let (_screen, x, y) = pointer.position();
    Some((x, y))
}

#[cfg(not(target_os = "linux"))]
fn cursor_position_gdk() -> Option<(i32, i32)> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Hyprland's `hyprctl -j monitors` output shape — array of monitor
    /// objects, the focused entry carries `focused: true` and full EDID
    /// metadata.
    #[test]
    fn focused_monitor_info_picks_hyprland_focused_entry() {
        let json = json!([
            {
                "name": "DP-1",
                "make": "Dell",
                "model": "U2723QE",
                "serial": "ABC123",
                "focused": false,
            },
            {
                "name": "DP-2",
                "make": "LG",
                "model": "27UP850",
                "serial": "XYZ789",
                "focused": true,
            },
        ]);
        let info = focused_monitor_info(&json, "focused").expect("focused entry exists");
        assert_eq!(info.name, "DP-2");
        assert_eq!(info.make.as_deref(), Some("LG"));
        assert_eq!(info.model.as_deref(), Some("27UP850"));
        assert_eq!(info.serial.as_deref(), Some("XYZ789"));
    }

    /// Sway's `swaymsg -t get_outputs -r` emits the same array shape.
    /// The `-r` flag returns the raw JSON instead of the IPC-wrapped form.
    #[test]
    fn focused_monitor_info_picks_sway_focused_entry() {
        let json = json!([
            {
                "name": "eDP-1",
                "make": "BOE",
                "model": "Built-in",
                "serial": null,
                "focused": true,
            },
            {
                "name": "HDMI-A-1",
                "focused": false,
            },
        ]);
        let info = focused_monitor_info(&json, "focused").expect("focused entry exists");
        assert_eq!(info.name, "eDP-1");
        assert_eq!(info.make.as_deref(), Some("BOE"));
        // null serial in JSON deserialises as None on string projection.
        assert_eq!(info.serial, None);
    }

    /// No entry with `focused: true` → None. Caller falls through to its
    /// own monitor-selection fallback (primary → any).
    #[test]
    fn focused_monitor_info_returns_none_when_nothing_focused() {
        let json = json!([
            { "name": "DP-1", "focused": false },
            { "name": "DP-2", "focused": false },
        ]);
        assert!(focused_monitor_info(&json, "focused").is_none());
    }

    /// Focused entry exists but has no `name` field — the projection
    /// short-circuits via `?` on `string_at("name")` and returns None
    /// rather than emitting a `MonitorInfo` with an empty connector.
    #[test]
    fn focused_monitor_info_returns_none_when_name_missing() {
        let json = json!([
            { "make": "Anonymous", "focused": true },
        ]);
        assert!(focused_monitor_info(&json, "focused").is_none());
    }

    /// Top-level shape isn't an array (e.g. compositor IPC returned an
    /// error envelope) — `as_array()?` short-circuits to None.
    #[test]
    fn focused_monitor_info_returns_none_for_non_array() {
        let json = json!({ "error": "no outputs" });
        assert!(focused_monitor_info(&json, "focused").is_none());
    }

    /// Spawning a binary that doesn't exist → `Command::output` errors
    /// with `ENOENT`; the function logs at debug and returns None.
    #[test]
    fn ipc_json_returns_none_on_spawn_failure() {
        let result = ipc_json("/nonexistent/binary/that/does/not/exist", &[]);
        assert!(result.is_none());
    }

    /// Binary runs successfully but emits non-JSON stdout → `from_slice`
    /// fails; the function logs at debug and returns None.
    #[test]
    fn ipc_json_returns_none_on_non_json_stdout() {
        // /bin/echo is POSIX-portable; emits "not-json\n" to stdout
        // exit code 0. The non-JSON parse path is what we want here.
        let result = ipc_json("/bin/echo", &["not-json"]);
        assert!(result.is_none());
    }
}
