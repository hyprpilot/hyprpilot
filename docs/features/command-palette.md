---
title: Command palette
order: 2
---

# Command palette

`Ctrl+K` opens the palette. Every captain-facing action lives behind a leaf — no menus, no settings dialogs, no mouse needed.

![palette root with all 11 leaves visible](/screenshots/palette-root.png)

## Root leaves

| Leaf | What it does |
| --- | --- |
| **instance** | Manage the currently focused instance (rename, restart, shutdown, change cwd, change profile). |
| **instances** | Switch focus between live instances. Spawn new ones. |
| **profiles** | Pick the default profile for new instances. |
| **sessions** | Resume a persisted ACP session for the current cwd / agent. |
| **models** | Pick a model for the focused instance (live `models_set`). |
| **modes** | Pick a mode (claude-code's `plan` / `default`, etc.). |
| **effort** | Pick a thinking budget / reasoning effort tier. |
| **cwd** | Change the focused instance's working directory (triggers a session restart). |
| **mcps** | Read-only catalog of the active instance's resolved MCP set. |
| **skills** | Pick a skill to attach to the next prompt. |
| **daemon** | Daemon ops — reload config, status snapshot, shut down. |

## Sessions leaf

![sessions leaf, cwd-filtered, with 4 results](/screenshots/palette-sessions.png)

The sessions leaf calls `session_list` on the daemon. Sessions are cwd-filtered by default — you only see sessions rooted at the focused instance's current directory. `Ctrl+D` on a row reveals the (currently stub) `sessions/forget` action.

## Models leaf

![models leaf with claude-haiku highlighted](/screenshots/palette-models.png)

Lists every model the agent advertises via `current_model_update` (claude-code surfaces sonnet / opus / haiku; codex surfaces its tier). Picking a row routes through `models_set`, which the agent confirms by re-emitting `current_model_update`. The header chip flips when the daemon receives that event — no optimistic UI.

## Keymap

| Keys | Action |
| --- | --- |
| `Ctrl+K` | Open the palette / cycle to next leaf-stack level |
| `Ctrl+P` | Open the palette pre-filtered (synonym) |
| `Esc` | Close the palette |
| `Tab` / `Shift+Tab` | Navigate within a leaf |
| `Enter` | Commit the highlighted row |
| `Ctrl+D` | Delete / forget (where supported, e.g. sessions) |
| Type to filter | Fuzzy match using nucleo (the same matcher path/ripgrep use) |

## Adding a leaf

Adding a new root leaf is one variant on `PaletteLeafId` + one entry in `ROOT_LEAVES` + one `open<Name>Leaf()` exporter. The `openRootLeaf` dispatcher's exhaustiveness check fails compile until both land — you can't ship a half-wired leaf.
