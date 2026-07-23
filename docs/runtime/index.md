---
title: What is hyprpilot
order: 1
prev: false
---

# {{ $frontmatter.title }}

Hyprpilot is a **config-driven, fire-and-exec launcher for terminal coding agents**, shipped as a single Rust binary. It resolves a session _profile_ from layered config, projects that profile onto the chosen vendor's **native** CLI flags and environment, and `exec()`s into the vendor CLI — `claude`, `codex`, or `opencode` — replacing its own process.

<!-- more -->

## The fire-and-exec model

A launch is one straight line:

1. **Resolve** — pick a profile (the positional `[PROFILE]` id, the configured default, or the interactive picker) and fold config layers, [`patches`](../config/patches), and [`--with-config`](./with-config) overlays onto it.
2. **Project** — translate the resolved profile (model, mode, system prompt, MCP catalogue, tool policy) onto the vendor's own flags and environment variables.
3. **Rename** — optionally retitle the current tmux window / zellij tab to `hyprpilot@<cwd>`.
4. **`exec()`** — replace the hyprpilot process with the vendor CLI. On unix there is no child process left behind; the vendor TUI simply _is_ your terminal from that point on.

::: info No daemon, no socket, no UI

There is **no background daemon, no unix socket, and no window or desktop UI** anywhere in hyprpilot. Once the vendor CLI is running, hyprpilot is gone — it inherits your shell environment, hands over the terminal, and its exit code is the vendor's.

:::

## The one long-lived thing

The single component that outlives the launch is the in-tree **MCP server** (`hyprpilot mcp serve`). When your resolved profile has a non-empty skills catalogue, the launcher auto-injects a stdio MCP entry named `hyprpilot` into the vendor's MCP config, and the vendor spawns that sidecar itself — so your `SKILL.md` catalogue reaches the agent over MCP. See [Skills & the hyprpilot MCP Server](./skills).

## Why you would want it

If you launch the same agent CLI with the same model, working directory, system prompt, and MCP servers every day, hyprpilot turns that incantation into a named profile:

```sh
hyprpilot engineer            # instead of a 200-character vendor invocation
```

If you switch between vendors, profiles keep each vendor's flag dialect out of your muscle memory — the same profile shape projects onto whichever provider the profile names.

## Where to go next

- [Installation](./installation) — AUR packages or a source build.
- [Quickstart](./quickstart) — a minimal config and your first launch.
- [Launching](./launch) — the launch flags, the picker, and the environment knobs.
- [Config](../config/) — the full configuration reference.
