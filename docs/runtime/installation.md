---
title: Installation
order: 10
---

# {{ $frontmatter.title }}

Hyprpilot is a single Rust binary. It resolves a session profile from layered config and `exec()`s into the vendor's native agent CLI — so the only hard runtime dependency is the vendor CLI you want to launch (`claude`, `codex`, or `opencode`).

<!-- more -->

## Arch

Hyprpilot is published to the AUR in two flavors. Pick one — they conflict by design.

::: code-group

```sh [hyprpilot-bin]
# Prebuilt binary — tracks the latest GitHub Release.
yay -S hyprpilot-bin
```

```sh [hyprpilot-git]
# Builds from the latest `main` with cargo.
yay -S hyprpilot-git
```

:::

Both install the binary, the terminal `.desktop` entry, and the hicolor icons. Swap `yay` for `paru` or your AUR helper of choice.

## Building from source

If you are not on an Arch-like distro, build with a stock Rust toolchain — it is a plain Rust build with no webkit / gtk / node dependency:

```sh
git clone https://github.com/hyprpilot/hyprpilot
cd hyprpilot
cargo build --release
install -Dm755 target/release/hyprpilot ~/.local/bin/hyprpilot
```

A manual build only drops the binary; the AUR packages also install the desktop entry and icons. See [Development](../repository/development) for the pinned toolchain and `task` targets.

## The desktop entry

The AUR packages install a terminal-launcher `.desktop` file at `/usr/share/applications/hyprpilot.desktop`:

```ini
[Desktop Entry]
Type=Application
Name=hyprpilot
GenericName=Agent CLI launcher
Comment=Resolves a profile and execs your coding agent's native CLI.
Exec=hyprpilot
Icon=hyprpilot
Categories=Development;Utility;
StartupNotify=false
Terminal=true
```

`Terminal=true` means any XDG app launcher (rofi, wofi, fuzzel, the GNOME grid) lists **hyprpilot**; selecting it opens a terminal, runs the interactive profile picker, and execs the chosen agent in that terminal. Nothing extra to wire — installing the package is enough.

## Launch from a compositor keybind

If you prefer a keybind over an app launcher, bind a key to a terminal that runs hyprpilot. Bare `hyprpilot` opens the interactive picker; `hyprpilot <id>` skips straight to a profile.

::: code-group

```ini [Hyprland]
# ~/.config/hypr/hyprland.conf
bind = SUPER, RETURN, exec, foot -e hyprpilot
bind = SUPER SHIFT, RETURN, exec, foot -e hyprpilot engineer
```

```sh [Sway]
# ~/.config/sway/config
bindsym $mod+Return exec foot -e hyprpilot
bindsym $mod+Shift+Return exec foot -e hyprpilot engineer
```

:::

Swap `foot` for your terminal of choice (`kitty -e`, `alacritty -e`, `wezterm start --`, `gnome-terminal --`, …). The vendor TUI takes over that terminal until you exit it.

## After install

1. **Install a vendor CLI** — hyprpilot launches `claude`, `codex`, or `opencode`; at least one must be on your `$PATH`.
2. **Configure at least one profile** — fresh installs ship agent defaults but **no** profile, and hyprpilot refuses to launch until one exists. The [Quickstart](./quickstart) walks you through it.

Because hyprpilot `exec()`s into the vendor CLI, it inherits your shell environment as-is — API keys, `$PATH`, and everything else the vendor needs are already present when you run it from a terminal. There is no long-lived process to keep hydrated.
