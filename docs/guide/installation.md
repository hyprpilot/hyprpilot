---
title: Installation
order: 1
---

# Installation

Hyprpilot is published to the AUR in two flavors. Pick one — they conflict by design.

## Arch

### Prebuilt binary — `hyprpilot-bin`

```sh
yay -S hyprpilot-bin
```

Tracks the latest GitHub Release.

### Source build — `hyprpilot-git`

```sh
yay -S hyprpilot-git
```

Builds from `main`.

## Compositor support

| Compositor  | Anchor mode | Center mode |
| ----------- | ----------- | ----------- |
| Hyprland    | ✅          | ✅          |
| Sway        | ✅          | ✅          |
| GNOME / KDE | ❌          | ✅          |
| X11         | ❌          | ✅          |

GNOME / KDE / X11: set `[daemon.window] mode = "center"` in your config.

## After install

1. **Configure profiles** — drop a `[[profiles]]` block in `~/.config/hyprpilot/config.toml`. See [Configuration → Profiles](../configuration/profiles).
2. **Pick how it starts** — keybind, systemd user unit, or `exec-once`. See [Integration](./integration).
3. _(Optional)_ **Status in the bar** — see [Waybar](./waybar).

## Running it

The daemon needs your desktop environment variables (`WAYLAND_DISPLAY`, `XDG_RUNTIME_DIR`, `DISPLAY`, `DBUS_SESSION_BUS_ADDRESS`, `PATH` extensions for `bunx` / `npx`, API keys, …) to spawn agents and bind the overlay to your compositor. Two paths:

### Run from your shell (development / one-off)

```sh
hyprpilot daemon
```

The shell already has your env loaded, so this just works. Pop the overlay with `hyprpilot ctl overlay toggle` from any terminal — see [Integration](./integration) for keybinds.

### As a systemd user service (recommended)

The AUR package installs a unit at `/usr/lib/systemd/user/hyprpilot.service`:

```sh
systemctl --user enable --now hyprpilot.service
```

The shipped unit is intentionally **not** bound to `graphical-session.target` — we don't make assumptions about your session manager. If you want the daemon to start with your graphical session, drop a override in `~/.config/systemd/user/hyprpilot.service.d/override.conf`:

```ini
[Unit]
After=graphical-session.target
PartOf=graphical-session.target

[Install]
WantedBy=graphical-session.target
```

Hyprland users can also follow the [systemd integration](https://wiki.hyprland.org/Useful-Utilities/Systemd-start/) docs to make `graphical-session.target` fire at the right moment.

Logs:

```sh
journalctl --user -u hyprpilot.service -f
```

If you launch the daemon another way (Hyprland's `exec-once`, a custom script), make sure the environment is hydrated first — wrap it in your shell:

```sh
exec-once = bash -lc 'hyprpilot daemon'
```

`bash -lc` reads your login files (`~/.bash_profile`, `~/.zprofile`, …) so API keys and `PATH` reach the daemon. Without that, `bunx` won't find your runtime and agent spawn fails silently.

```sh
hyprpilot --help
hyprpilot ctl --help
```
