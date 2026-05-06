---
title: Installation
order: 1
---

# Installation

Hyprpilot is published to the AUR in two flavors. Pick one — they conflict by design.

## Arch (recommended)

### Prebuilt binary — `hyprpilot-bin`

```sh
yay -S hyprpilot-bin
# or paru -S hyprpilot-bin
```

Tracks the latest GitHub Release. Zero build time, ~5 MB download.

### Source (VCS) — `hyprpilot-git`

```sh
yay -S hyprpilot-git
```

Builds from `main`. Pulls Cargo + Node + pnpm + system Webkit deps as makedepends. ~10 minutes on a modern machine.

## System dependencies

Both packages declare these — pacman pulls them automatically. If you build outside the AUR (e.g. `cargo build`), install them yourself:

| Dep | What it provides |
| --- | --- |
| `webkit2gtk-4.1` | Webview the overlay renders into. |
| `gtk3` | GTK toolkit (until [Tauri's GTK4 port](https://github.com/tauri-apps/wry/pull/1530) lands upstream). |
| `gtk-layer-shell` | `zwlr_layer_shell_v1` integration for the anchor-mode surface. |
| `libappindicator-gtk3` | Tray icon. |

## Compositor support

| Compositor | Anchor mode | Center mode |
| --- | --- | --- |
| Hyprland | ✅ | ✅ |
| Sway | ✅ | ✅ |
| GNOME / KDE | ❌ (no `zwlr_layer_shell_v1`) | ✅ |
| X11 | ❌ | ✅ |

GNOME / KDE captains: set `[daemon.window] mode = "center"` in your config.

## After install

1. **Configure agents.** Drop a `[[profiles]]` block in `~/.config/hyprpilot/config.toml` — see [Configuration → Agents](../configuration/agents).
2. **Wire keybinds.** Bind `hyprpilot ctl overlay toggle` somewhere reachable — see [Hyprland integration](./hyprland).
3. **(Optional) Waybar status.** Add the `custom/hyprpilot` module — see [Waybar integration](./waybar).
4. **(Optional) Autostart.** Set `[autostart] enabled = true` or use `exec-once` — see [Autostart](./autostart).

## First run

```sh
hyprpilot daemon
```

The daemon boots hidden by default. Pop the overlay via your keybind, or run `hyprpilot ctl overlay toggle` from any terminal.

```sh
hyprpilot --help
hyprpilot ctl --help
```
