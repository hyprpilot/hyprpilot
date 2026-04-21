# Waybar integration

hyprpilot exposes a live status stream via `ctl status --watch` that waybar's
`custom/*` module protocol can consume directly. Clicking the indicator toggles
the overlay via `ctl toggle` — no extra wiring needed.

## Waybar config

Add the following to your waybar `config` file (usually
`~/.config/waybar/config`). Adjust the position in your `modules-left` /
`modules-center` / `modules-right` list to taste.

```jsonc
"custom/hyprpilot": {
    "exec": "hyprpilot ctl status --watch",
    "return-type": "json",
    "on-click": "hyprpilot ctl toggle",
    "escape": true,
    "restart-interval": 5
}
```

### How it works

- `exec` runs `ctl status --watch`, which connects to the daemon, calls
  `status/subscribe`, and streams one JSON object per state change to stdout.
  Waybar re-renders on each line.
- `return-type: "json"` tells waybar to parse the line as
  `{ text, class, tooltip, alt, ... }`.
- `on-click` calls `ctl toggle` — the CLI round-trips through the JSON-RPC
  socket and flips window visibility.
- `restart-interval: 5` is a safety net: if `ctl status --watch` exits
  (e.g. the daemon is killed), waybar restarts it after 5 seconds. The
  `--watch` client itself reconnects with back-off on socket loss and emits an
  `"offline"` payload between attempts, so the indicator always shows
  something valid.

### One-shot polling (alternative)

If you prefer polling over a persistent connection, omit `--watch`:

```jsonc
"custom/hyprpilot": {
    "exec": "hyprpilot ctl status",
    "return-type": "json",
    "on-click": "hyprpilot ctl toggle",
    "interval": 2
}
```

One-shot mode exits 0 even if the daemon is not running — it emits an
`"offline"` payload instead of an error so waybar never shows a broken pipe
warning.

## CSS styling

Waybar applies the `class` field as a CSS class on the widget. Style by state:

```css
#custom-hyprpilot {
    color: #6c7086; /* idle / default */
    padding: 0 8px;
}

#custom-hyprpilot.streaming {
    color: #a6e3a1; /* green — agent responding */
}

#custom-hyprpilot.awaiting {
    color: #f9e2af; /* yellow — waiting for input */
}

#custom-hyprpilot.error {
    color: #f38ba8; /* red — last session errored */
}

#custom-hyprpilot.offline {
    color: #45475a; /* dim — daemon not running */
}
```

## State reference

| `state` | `text` | `class` | `tooltip` |
| ------- | ------ | ------- | --------- |
| `idle` | *(empty)* | `idle` | `hyprpilot: idle` |
| `streaming` | `●` | `streaming` | `hyprpilot: agent is responding` |
| `awaiting` | `?` | `awaiting` | `hyprpilot: awaiting input` |
| `error` | `!` | `error` | `hyprpilot: last session errored` |
| `offline` | *(empty)* | `offline` | `hyprpilot: offline` |

`alt` always mirrors `state`, so you can use it with waybar's `format-alt`
or icon mapping if you prefer glyphs over text.

## Requirements

- hyprpilot daemon running (`hyprpilot daemon` or via a systemd user unit).
- `hyprpilot` binary in `$PATH` (or use the full path in `exec`/`on-click`).
- waybar 0.9.24+ (for stable `return-type: "json"` support).
