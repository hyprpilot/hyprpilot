---
title: Window
order: 4
---

# Window

The overlay runs in one of two modes. Pick by setting `[daemon.window] mode`.

## Anchor mode (Hyprland / Sway)

A surface pinned to a screen edge, painted above normal windows.

```toml
[daemon.window]
mode = "anchor"
output = "DP-1"        # optional — defaults to the focused monitor
visible = false        # boot hidden; show via keybind / tray / `ctl overlay toggle`

[daemon.window.anchor]
edge = "right"         # "top" | "right" | "bottom" | "left"
margin = 0             # px from the anchored edge
width = "40%"          # "N%" of monitor, or pixel int
# height unset         # unset = full-height fill
```

`width` / `height` accept either a pixel integer or `"N%"` of the active monitor. Percentages re-resolve every time the overlay shows, so moving it between monitors gives the right size for the new screen.

Leave `height` unset for a full-height stripe. Set it for a fixed-height panel.

## Center mode (GNOME / KDE / X11)

A regular window centered on the active monitor. Works on any compositor.

```toml
[daemon.window]
mode = "center"

[daemon.window.center]
width = "50%"
height = "60%"
```

## Monitor selection

If `[daemon.window] output` is set, the overlay always anchors to that monitor. Otherwise it follows the focused monitor (Hyprland and Sway report which one that is via their IPC; on other compositors, the cursor's monitor is used).

If neither resolves, the compositor's primary monitor is the fallback.

## Hidden by default

`visible = false` means the overlay is configured at boot but starts unmapped. First show happens via:

- A Hyprland keybind (e.g. `bind = SUPER, space, exec, hyprpilot ctl overlay toggle`)
- The tray icon
- `hyprpilot ctl overlay toggle` from any terminal
- Running `hyprpilot` a second time (escape hatch when no keybind is bound yet)

Set `visible = true` to keep the overlay on at boot.

## Zoom

```toml
[ui]
zoom = 1.0   # range [0.5, 2.0]
```

Scales everything uniformly — paddings, widths, fonts. Works the same on every compositor and OS. Use this knob for retina displays or just to bump the chrome up.
