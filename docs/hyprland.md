# Hyprland integration

hyprpilot exposes an `overlay/*` RPC namespace that maps cleanly onto a
hyprland keybind. The recommended binding flips the overlay with a single
chord.

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

Every `overlay/*` call serialises through `WindowRenderer::lock_present`
on the daemon side, so two near-simultaneous keybind taps land in a
deterministic visible-XOR-hidden state — never "both hide" or "both
show".

## How it works

- `overlay/toggle` is the canonical bind target — no params, single
  round-trip, returns `{"visible": bool}` reflecting the post-toggle
  state.
- `overlay/present` takes an optional `instanceId` and routes through
  the same `Adapter::focus` path the UI uses, so a binding like
  `bind = SUPER, 1, exec, hyprpilot ctl overlay present --instance <uuid>`
  brings the overlay forward AND switches to a specific instance in
  one chord.
- `overlay/hide` keeps the webview alive (just unmaps the surface) so
  the next present is instant; the daemon process doesn't restart.

## Why not `window/toggle`?

`window/toggle` predates the overlay namespace and is the surface waybar
uses (`on-click: hyprpilot ctl toggle`). The two namespaces serve
different consumers and carry different parameters (`overlay/*` accepts
`instanceId`); both stay live — `overlay/*` is the recommended target
for new keybinds.
