---
title: Autostart
order: 4
---

# Autostart

Hyprpilot can launch at user login on Linux desktop environments (GNOME,
KDE), macOS, and Windows. Enabled with one config knob.

## Enabling

```toml
[autostart]
enabled = true
```

Restart the daemon once. On next boot it registers the appropriate
login entry for your platform (XDG autostart on Linux DEs, LaunchAgent
on macOS, Registry `Run` key on Windows).

Setting `enabled = false` removes the entry on next boot.

## Hidden-on-boot default

`[daemon.window] visible = false` (the default) — the daemon boots with
the overlay hidden. First show happens via a Hyprland keybind, the tray
icon, or `hyprpilot ctl overlay toggle` from any terminal.

Set `visible = true` to keep the overlay on at boot.

## Hyprland users — read this

Hyprland and Sway don't fire XDG autostart entries — that's a
desktop-environment feature. `[autostart] enabled` silently no-ops
there.

Use one of these instead:

1. Add `exec-once = hyprpilot` to your `~/.config/hypr/hyprland.conf`.
2. After installing via the AUR package, enable the user unit:
   `systemctl --user enable --now hyprpilot.service`.

## Tray icon

The daemon installs a system tray icon at boot. Click → toggle
overlay. Right-click for a menu:

- **Toggle overlay** — same as left-click.
- **Show overlay** — explicit show (no-op when already visible).
- **Hide overlay** — explicit hide (no-op when already hidden).
- **Shut down** — clean shutdown.

If no system tray is available (some minimal compositors), the daemon
logs a warning and continues without one — the keybind / `ctl` paths
still work.
