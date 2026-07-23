---
title: Integration
order: 2
---

# Integration

Hyprpilot is fire-and-exec: it runs in the foreground of a terminal,
resolves a profile, and replaces itself with the vendor CLI. Integration
is therefore about *how you open that terminal* — a keybind, an
app-launcher entry, or the shipped `.desktop` file — plus the
multiplexer niceties that make agent panes easy to spot.

## Launch from a compositor keybind

Bind a key to a terminal that runs hyprpilot. Bare `hyprpilot` opens the
interactive picker; `hyprpilot -p <id>` skips straight to a profile.

```ini
# Hyprland — ~/.config/hypr/hyprland.conf
bind = SUPER, RETURN, exec, foot -e hyprpilot
bind = SUPER SHIFT, RETURN, exec, foot -e hyprpilot -p engineer
```

```bash
# Sway — ~/.config/sway/config
bindsym $mod+Return exec foot -e hyprpilot
bindsym $mod+Shift+Return exec foot -e hyprpilot -p engineer
```

Swap `foot` for your terminal of choice (`kitty -e`, `alacritty -e`,
`wezterm start --`, `gnome-terminal --`, …). The vendor TUI takes over
that terminal until you exit it.

## App launcher / desktop entry

The AUR packages install a terminal-launcher `.desktop` at
`/usr/share/applications/hyprpilot.desktop` (`Terminal=true`,
`Exec=hyprpilot`). Any XDG app launcher (rofi, wofi, fuzzel, the GNOME
grid) will list **hyprpilot**; selecting it opens a terminal, runs the
interactive picker, and execs the chosen agent. Nothing extra to wire —
installing the package is enough.

## Multiplexer window titles

When hyprpilot launches inside tmux or zellij, it renames the current
window / tab to `hyprpilot@<cwd-basename>` right before `exec()` so you
can tell agent panes apart at a glance. It is on by default:

```toml
[multiplexer]
set_title = true          # default; set false to opt out
```

It is best-effort — outside a multiplexer it is a no-op, and any failure
is logged at `debug` and never aborts the launch. The rename shells out
to `tmux rename-window` / `zellij action rename-tab`, not raw OSC escape
sequences, so it respects your multiplexer's own title settings.

## Running a specific working directory

`--cwd` sets where the vendor process starts:

```sh
hyprpilot -p engineer --cwd ~/code/hyprpilot
```

cwd precedence is: explicit `--cwd` flag → the profile's (or agent's)
configured `cwd` → the current directory. So a profile pinned to a repo
launches there by default, and `--cwd` overrides it per invocation. See
[Configuration → Profiles](../configuration/profiles).

## Forwarding native arguments

Everything after a `--` separator is forwarded verbatim to the vendor
CLI — use it for provider-native flags and resume flows:

```sh
hyprpilot -p engineer -- --resume
hyprpilot -p review -- --model claude-opus-4-5
```
