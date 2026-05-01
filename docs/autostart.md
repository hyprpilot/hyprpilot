# Autostart

hyprpilot can launch at user login on Linux DEs (GNOME, KDE), macOS,
and Windows via the cross-platform
[`tauri-plugin-autostart`](https://tauri.app/plugin/autostart/). The
captain enables it through one config knob; the daemon reconciles the
OS-side autostart entry on every boot.

## Enabling

```toml
[autostart]
enabled = true
```

Restart the daemon once after editing. On next boot the daemon writes
the appropriate autostart entry per platform:

| Platform | Mechanism | Path |
| --- | --- | --- |
| Linux DE | XDG `.desktop` autostart | `~/.config/autostart/hyprpilot.desktop` |
| macOS | launchd LaunchAgent | `~/Library/LaunchAgents/com.hyprpilot.hyprpilot.plist` |
| Windows | Registry `Run` key | `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\hyprpilot` |

Setting `enabled = false` removes the entry on next boot.

## Hidden-on-boot default

`[daemon.window] visible = false` (the default) — daemon boots with the
overlay surface configured but unmapped. First user-visible map happens
through any of:

- a Hyprland keybind (e.g. `bind = SUPER, space, exec, hyprpilot ctl overlay toggle`);
- the system tray icon's left-click or "Show overlay" menu item;
- the bare `hyprpilot` invocation (a second invocation of a running
  daemon with no subcommand pops the overlay — captain's CLI escape
  hatch when no keybind is bound yet);
- the `overlay/present` RPC.

Set `visible = true` to glue the overlay on at boot — useful for the
"keep-it-pinned" workflow.

## Hyprland users — read this

`tauri-plugin-autostart` on Linux writes an XDG `.desktop` autostart
file. **wlroots-based compositors (Hyprland, Sway) don't fire XDG
autostart entries** — that's a desktop-environment feature. The
plugin's path silently no-ops on Hyprland.

Two options:

1. **Until AUR packaging lands**: add `exec-once = hyprpilot` to your
   `~/.config/hypr/hyprland.conf`. The `[autostart] enabled` config
   knob isn't load-bearing on Hyprland — `exec-once` is.
2. **After AUR packaging lands**: install via `pacman -S hyprpilot`
   (or AUR equivalent), then `systemctl --user enable --now hyprpilot.service`.
   The systemd user unit fires on `graphical-session.target` which
   Hyprland imports its environment into. `[autostart] enabled` stays
   the cross-platform knob; the systemd unit is the recommended
   Linux runtime path.

## Tray icon

The daemon installs a system tray icon at boot. Click → toggle
overlay. Right-click for a menu:

- **Toggle overlay** — same as left-click.
- **Show overlay** — explicit show (no-op when already visible).
- **Hide overlay** — explicit hide (no-op when already hidden).
- **Shut down** — clean shutdown via the same path as
  `hyprpilot ctl daemon shutdown` and `SIGTERM`.

If no system tray is available (some minimal compositors), the daemon
logs a warning and continues without one — the keybind / `ctl` paths
still work.
