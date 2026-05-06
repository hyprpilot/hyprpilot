---
title: Command palette
order: 2
---

# Command palette

`Ctrl+K` opens the palette. Every action lives behind a leaf — no menus, no settings dialogs, no mouse needed.

![palette root with all 11 leaves visible](/screenshots/palette-root.png)

## Root leaves

| Leaf | What it does |
| --- | --- |
| **instance** | Manage the focused instance — rename, shutdown. |
| **instances** | Switch focus between live instances; spawn new ones. |
| **profiles** | Pick the default profile for new instances. |
| **sessions** | Resume a previously-saved chat session. |
| **models** | Pick a model for the focused instance. |
| **modes** | Pick a mode (e.g. claude-code's `plan` / `default`). |
| **effort** | Pick a thinking budget / reasoning effort tier. |
| **cwd** | Change the focused instance's working directory. |
| **mcps** | View the active MCP set for the focused instance. |
| **skills** | Reload the skills catalog. |
| **daemon** | Daemon ops — reload config, shut down. |

## Sessions

![sessions leaf, cwd-filtered, with 4 results](/screenshots/palette-sessions.png)

Picks up where you left off. Sessions are filtered by the current working directory — you only see sessions rooted there.

## Models

![models leaf with available options](/screenshots/palette-models.png)

Lists every model the agent advertises. Pick a row, the header model chip flips once the agent confirms.

## Modes & Effort

![modes leaf with the alternative mode](/screenshots/palette-modes.png)

Modes are vendor-defined. claude-code ships `plan` + `default`. The currently-active mode is hidden so the leaf shows the alternatives only.

![effort leaf with low / medium / extra high / max options](/screenshots/palette-effort.png)

Effort is claude-code's adaptive-thinking budget — `low` / `medium` / `high` / `xhigh` / `max`. Higher = deeper reasoning, slower.

## Instance & Instances

![instance leaf — new / rename / shutdown actions](/screenshots/palette-instance.png)

Single-instance actions: `new` stages a fresh instance; `rename` gives it a captain-friendly name; `shutdown` tears it down.

![instances leaf — master-detail with 3 live instances](/screenshots/palette-instances.png)

The instance switcher. List on the left, preview on the right — adapter / model / mode + recent transcript. `Ctrl+D` shuts down a row.

## MCPs & Skills

![mcps catalog — master-detail with 4 servers and a raw JSON disclosure](/screenshots/palette-mcps.png)

Read-only view of the MCPs wired to the focused instance. Each server's preview shows source file, command, and the auto-accept / auto-reject globs. To change the set, edit the source JSON and restart the daemon.

![skills leaf — reload action](/screenshots/palette-skills.png)

To attach a skill to your next prompt, use the composer's `#<name>` sigil. The skills leaf has one action — `reload` — for picking up changes after editing a `SKILL.md`.

## Cwd

![cwd leaf — empty input prompt](/screenshots/palette-cwd.png)

Type a path; the focused instance restarts in the new directory. `Tab` autocompletes against your filesystem.

## Daemon

![daemon leaf — reload + shutdown actions](/screenshots/palette-daemon.png)

Daemon-level ops. `reload` re-reads config, MCPs, and skills. `shutdown` exits cleanly.

## Keymap

| Keys | Action |
| --- | --- |
| `Ctrl+K` | Open the palette |
| `Esc` | Close the palette |
| `↑` / `↓` | Navigate rows |
| `Enter` | Commit the highlighted row |
| `Ctrl+D` | Delete / shut down (where supported) |
| Type to filter | Fuzzy match |
