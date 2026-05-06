---
title: Integration
order: 2
---

# Integration

Wire hyprpilot into your compositor: keybinds, autostart, the tray icon. Examples below cover Hyprland and Sway — both ride the same `hyprpilot ctl` surface.

## Keybinds

The recommended chord flips the overlay on and off:

```ini
# Hyprland — ~/.config/hypr/hyprland.conf
bind = SUPER, space, exec, hyprpilot ctl overlay toggle
```

```bash
# Sway — ~/.config/sway/config
bindsym $mod+space exec hyprpilot ctl overlay toggle
```

Concurrent calls are race-safe — two near-simultaneous taps land in a deterministic state, never "both hide" or "both show".

## Subcommands worth binding

```sh
# Show + focus the overlay (no-op when already visible).
hyprpilot ctl overlay present

# Show + focus a specific instance in one chord.
hyprpilot ctl overlay present --instance <uuid>

# Hide. Webview stays warm; next show is instant.
hyprpilot ctl overlay hide

# Flip.
hyprpilot ctl overlay toggle
```

## Push-to-talk pattern

Pipe text into `hyprpilot ctl prompts send` from anywhere — voice transcription, clipboard, scratchpad — and the overlay fires the prompt:

```ini
# Hyprland — dump a voice transcription into the focused instance
bind = SUPER, grave, exec, whisper-stream | hyprpilot ctl prompts send --stdin
```

The same shape works with `wl-paste`, `xsel`, `xdotool`, or any program that emits text on stdout. A simple "send my clipboard as a prompt":

```bash
bindsym $mod+v exec wl-paste | hyprpilot ctl prompts send --stdin
```

## Autostart

The AUR package installs a systemd user unit at `/usr/lib/systemd/user/hyprpilot.service`. Enable it once:

```sh
systemctl --user enable --now hyprpilot.service
```

The shipped unit isn't bound to any session target — start condition is yours to compose. To tie it to your graphical session, see the [systemd note in the installation guide](./installation#as-a-systemd-user-service-recommended). Pairs with `[daemon.window] visible = false` (the default) so the daemon boots hidden and your keybind is the first user-visible map.

If you'd rather start it inline in your compositor config, wrap it so env vars come in:

```ini
# Hyprland
exec-once = bash -lc 'hyprpilot daemon'
```

```bash
# Sway
exec bash -lc 'hyprpilot daemon'
```

The cross-platform `[autostart] enabled = true` config knob registers an XDG autostart entry — that's the GNOME / KDE / macOS / Windows path. wlroots compositors (Hyprland, Sway) ignore XDG autostart, so use the systemd unit or `exec-once` there.

## Tray icon

The daemon installs a system tray icon at boot:

- **Left-click** — toggle overlay.
- **Right-click → Toggle / Show / Hide overlay** — same as the `ctl overlay` subcommands.
- **Right-click → Shut down** — clean shutdown.

If your compositor has no tray support, the daemon logs a warning and continues without one — the keybind and `ctl` paths still work.

## CLI escape hatch

Running `hyprpilot` (no subcommand) a second time, against a daemon that's already up, pops the overlay forward instead of starting a new daemon. Handy when nothing's bound yet:

```sh
hyprpilot       # first call — starts the daemon
hyprpilot       # second call — toggles overlay
```
