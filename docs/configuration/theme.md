---
title: Theme
order: 3
---

# Theme

Every color in the overlay is overridable from your config. Drop a `[ui.theme.*]` block in `~/.config/hyprpilot/config.toml`; everything you don't touch falls through to the default.

## Token groups

| Group | Fields | What it paints |
| --- | --- | --- |
| `font` | `mono`, `sans` | Monospace stack for chrome; sans stack for inline assistant prose. |
| `window` | `default`, `edge` | Window background + edge accent stripe. |
| `surface` | `default`, `bg`, `alt`, `card.{user,assistant}`, `compose`, `text` | Cards, composer, message bodies. |
| `fg` | `default`, `ink_2`, `dim`, `faint` | Text per emphasis level. |
| `border` | `default`, `soft`, `focus` | Separators + focus rings. |
| `accent` | `default`, `user`, `user_soft`, `assistant`, `assistant_soft` | Brand + per-speaker accents. |
| `state` | `idle`, `stream`, `pending`, `awaiting`, `working` | Live status indicators. |
| `kind` | `read`, `write`, `bash`, `search`, `agent`, `think`, `terminal`, `acp` | Per-tool-family colors. |
| `status` | `ok`, `warn`, `err` | Toast / banner hues. |
| `permission` | `bg`, `bg_active` | Permission row + modal panel. |

## Override example

To paint user message cards in a different blue:

```toml
[ui.theme.surface.card.user]
bg = "#1e3a5f"
```

That single field overrides; every other token stays at its default.

## Validation

Hex colors must be 6 or 8 characters (`#rrggbb` or `#rrggbbaa`). A typo aborts startup with a readable error pointing at the bad field.

## Fonts

Default to the system stack. On Linux the sans family follows your GTK font setting so chat prose matches the desktop. Monospace stays on the configured stack — code deserves a fixed-width font regardless of the desktop font.
