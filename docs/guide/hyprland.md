---
title: Hyprland
order: 2
---

# Hyprland integration

Bind a Hyprland chord to flip the overlay on and off.

## Recommended keybind

Add the following to your hyprland config (usually `~/.config/hypr/hyprland.conf`):

```ini
bind = SUPER, space, exec, hyprpilot ctl overlay toggle
```

`SUPER + space` is a suggestion — pick whatever fits your existing chord
layout.

## Subcommands

```sh
# Show + focus the overlay (no-op when already visible).
hyprpilot ctl overlay present

# Show + focus the overlay AND focus a specific instance.
hyprpilot ctl overlay present --instance <uuid>

# Hide the overlay (no-op when already hidden). Webview stays warm.
hyprpilot ctl overlay hide

# Flip visibility. Race-safe across concurrent calls.
hyprpilot ctl overlay toggle
```

Concurrent calls are race-safe — two near-simultaneous keybind taps
land in a deterministic state, never "both hide" or "both show".

## Notes

- `toggle` is the canonical bind target.
- `present --instance <uuid>` brings the overlay forward AND switches
  to a specific instance in one chord.
- `hide` keeps the webview alive (just unmaps the surface) so the next
  present is instant.
