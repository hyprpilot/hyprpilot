---
title: Window
order: 2
---

# Window

The daemon's overlay window runs in one of two modes. Pick by setting `[daemon.window] mode`.

## Anchor mode (Hyprland / Sway)

A `zwlr_layer_shell_v1` surface pinned to a screen edge, painted above normal windows. The Python-pilot shape.

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

`width` / `height` accept either a pixel integer or `"N%"` of the active monitor. Percentages resolve **per show transition**, so moving the overlay between monitors and toggling produces correctly-sized output for the new screen.

`height` unset is the default — the surface stretches full-height between top + bottom anchors. Setting an explicit `height` pins one edge with that fixed extent.

## Center mode (GNOME / KDE / X11)

A regular Tauri top-level window centered on the active monitor. Works on any compositor.

```toml
[daemon.window]
mode = "center"

[daemon.window.center]
width = "50%"
height = "60%"
```

## Monitor selection

When `[daemon.window] output` is set, the daemon pins to that connector. Otherwise it picks the **focused monitor**:

| Compositor | Source |
| --- | --- |
| Hyprland | `hyprctl -j monitors` → `focused: true` |
| Sway | `swaymsg -t get_outputs -r` → `focused: true` |
| Other | GDK pointer position + monitor bounds |

If the focused monitor can't be resolved, the primary monitor (compositor-defined) is used.

## Hidden by default

`visible = false` means the overlay surface is configured at boot but unmapped. First user-visible map happens via:

- Hyprland keybind (`bind = SUPER, space, exec, hyprpilot ctl overlay toggle`)
- Tray icon click
- `hyprpilot ctl overlay toggle` from any terminal
- A bare second `hyprpilot` invocation (single-instance escape hatch)

Set `visible = true` to glue the overlay on at boot.

## UI scaling

```toml
[ui]
zoom = 1.0   # range [0.5, 2.0]
```

Chromium-style page zoom — scales **everything** (paddings, widths, borders, fonts) uniformly. Cross-platform; works the same on every compositor.

A CSS `:root { font-size }` knob would only scale `rem`-based primitives; the codebase mixes `rem` typography with `px` paddings and would look broken under that approach. `set_zoom` is the canonical way.

## What's NOT exposed

Two layer-shell knobs are intentionally hardcoded:

- `layer = overlay` — anything else is a footgun for a chat overlay.
- `keyboard_interactivity = on_demand` — compose input needs to accept focus, but the overlay must not grab keys while idle.

If you want either changed, open an issue with the use case.
