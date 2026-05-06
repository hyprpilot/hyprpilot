---
title: Theme
order: 3
---

# Theme

The palette lives in Rust, not CSS. Defaults are seeded from `src-tauri/src/config/defaults.toml`; user TOMLs override any subset. The Tauri `get_theme` command serves the resolved tree to the webview, which writes every leaf onto `:root` as a `--theme-<path>` CSS custom property before the first render.

## Token groups

| Group | Fields | Purpose |
| --- | --- | --- |
| `font` | `mono`, `sans` | Monospace stack for chrome; sans stack for inline assistant prose. |
| `window` | `default`, `edge` | Top-level body bg + edge accent stripe. |
| `surface` | `default`, `bg`, `alt`, `card.{user,assistant}`, `compose`, `text` | Filled surfaces (cards, composer, message body). |
| `fg` | `default`, `ink_2`, `dim`, `faint` | Text colors per emphasis level. |
| `border` | `default`, `soft`, `focus` | Stroke colors for separators + focus rings. |
| `accent` | `default`, `user`, `user_soft`, `assistant`, `assistant_soft` | Brand + per-speaker accents. |
| `state` | `idle`, `stream`, `pending`, `awaiting`, `working` | Five-phase lifecycle colors driving live indicators. |
| `kind` | `read`, `write`, `bash`, `search`, `agent`, `think`, `terminal`, `acp` | Per-tool-family dispatch colors keyed by `ToolCall.kind`. |
| `status` | `ok`, `warn`, `err` | Toast / banner notification hues. |
| `permission` | `bg`, `bg_active` | Warm-brown panel fills for the permission stack. |

## Override example

To paint user message cards in a different blue:

```toml
[ui.theme.surface.card.user]
bg = "#1e3a5f"
```

That single field overrides; every other token falls through to the default.

## Validation

Hex color fields use a `HexColor` newtype — `#[0-9a-fA-F]{6,8}`. A typo'd color rejects at TOML parse time, not at runtime.

## CSS variable naming

The webview emits each token as a `--theme-<path>` custom property. Path segments named `default` or `bg` drop from the variable name (they represent the group's primary role):

| Token path | CSS variable |
| --- | --- |
| `fg.default` | `--theme-fg` |
| `surface.card.user.bg` | `--theme-surface-card-user` |
| `accent.assistant_soft` | `--theme-accent-assistant-soft` |

User stylesheets shouldn't redeclare theme values — Rust is the sole source. The only literal that lives outside Rust is the Tauri window's pre-mount `backgroundColor` (in `src-tauri/tauri.conf.json`); keep that equal to `[ui.theme.window] default` if you override it.

## UI font

Pulled from the system stack by default. On Linux, `useTheme.applyGtkFont` overrides the sans family with the captain's GTK font name so prose matches the desktop. Mono stays on the configured monospace stack — code deserves that regardless of desktop font.
