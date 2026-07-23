---
title: Quickstart
order: 20
---

# {{ $frontmatter.title }}

A fresh install already knows the three vendors — the compiled defaults seed `agents` entries for `claude-code`, `codex`, and `opencode` — but ships **zero** profiles, and hyprpilot refuses to launch until you configure one. This page gets you from nothing to a working launch.

<!-- more -->

## One profile

Create `~/.config/hyprpilot/config.yaml`:

```yaml
profile:
  default: engineer # which profile bare `hyprpilot` picks

profiles:
  - id: engineer
    agent: claude-code # references a seeded agents id
    model: claude-sonnet-4-5 # optional; profile > agent > vendor default
```

That is the entire minimum: one `profiles` entry pointing at a built-in agent, and a `profile.default` naming it.

::: tip YAML, TOML, or JSON

The docs write config in YAML, the recommended format. TOML and JSON work exactly the same — drop a `config.toml` / `config.json` instead. See [Config → Formats](../config/#formats).

:::

## Launch

```sh
hyprpilot                     # resolves the default profile, then execs `claude`
hyprpilot engineer            # or address the profile explicitly (positional)
```

Hyprpilot resolves the profile, projects it onto the vendor's native flags (here: `claude --model claude-sonnet-4-5`), and `exec()`s — the vendor TUI replaces hyprpilot in your terminal.

If you add more profiles and drop the `default`, bare `hyprpilot` opens an interactive fuzzy picker over them instead.

## Grow the profile

Everything a launch needs hangs off the same `profiles` entry — working directory, system prompts, MCP servers:

```yaml
profiles:
  - id: engineer
    agent: claude-code
    model: claude-sonnet-4-5
    cwd: ~/code/my-project
    system_prompt:
      - file: ~/.config/hyprpilot/prompts/engineer.md
    mcps:
      - file: ~/.claude.json
```

See [Config → Profiles](../config/profiles) for the full override surface.

## Check your work

```sh
hyprpilot profiles
```

lists every configured profile — the default marker, id, agent, and model — without launching anything. If your config has a typo, this is where you see the validation error naming the offending field.
