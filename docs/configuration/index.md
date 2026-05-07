---
title: Configuration overview
order: 1
---

# Configuration

Hyprpilot reads layered TOML. Each source overrides the one before it, so you only write what you want to change:

1. **Built-in defaults** — every knob has a working default.
2. **Global** — `~/.config/hyprpilot/config.toml`.
3. **Per-profile overlay** — `~/.config/hyprpilot/profiles/<name>.toml`, picked with `--config-profile <name>` or `HYPRPILOT_CONFIG_PROFILE=<name>`.
4. **CLI flags** — override-per-invocation; never persisted.

A minimal config that does anything useful only needs `[[agents]]` + `[[profiles]]`. Everything else is optional.

```toml
[[agents]]
id = "claude-code"
provider = "acp-claude-code"

[agents.env]
ANTHROPIC_API_KEY = "${env:ANTHROPIC_API_KEY}"

[[profiles]]
id = "ask"
agent = "claude-code"
model = "claude-haiku-4-5"
```

## Two `profile` concepts

The word "profile" lives in two parallel namespaces:

| Concept | Where | Purpose |
| --- | --- | --- |
| Config-layer profile | `--config-profile` CLI flag | Pick a different config TOML overlay (e.g. `work` vs `personal`). |
| Session profile | `[[profiles]]` blocks in TOML | Pick which agent + model + cwd + system prompt a chat instance uses. |

This page documents the layering. For session profiles — by far the most important config option — see [Profiles](./profiles).

## Where things live

| Path | What |
| --- | --- |
| `~/.config/hyprpilot/config.toml` | Global config. |
| `~/.config/hyprpilot/profiles/*.toml` | Config-layer overlays. |
| `~/.config/hyprpilot/skills/<slug>/SKILL.md` | Skill bundles. |
| `~/.config/hyprpilot/mcps/*.json` | MCP catalog files. |
| `~/.local/state/hyprpilot/logs/hyprpilot.log.*` | Rolling logs. |
| `$XDG_RUNTIME_DIR/hyprpilot.sock` | Daemon socket (one per user). |

## Root-level options

A handful of knobs live at the TOML root rather than under a section.

```toml
cwd = "~/code"           # default daemon working directory
```

`cwd` sets the daemon's working directory at startup when `--cwd` isn't passed on the CLI. Useful for systemd-unit invocations where there's no shell-set cwd. `--cwd` always wins; without either, the daemon inherits the spawning environment's cwd.

The `[[skills]]` and `[[mcps]]` array-of-tables also live at root — see [Profiles](./profiles).

## Validation

Every section validates types and rejects unknown fields at boot — typos fail fast with an error naming the offending field. Cross-field references (e.g. `[[profiles]].agent` must reference a real `[[agents]].id`) are checked too.

## Pages in this section

- [Profiles](./profiles) — the heart of the config: agents, models, cwd, system prompts, MCPs.
- [Agents](./agents) — registering agent vendors (claude-code, codex, opencode, custom).
- [Window](./window) — anchor vs center mode, monitor selection, sizing.
- [Theme](./theme) — every color in the overlay.
- [Remote bridge](./remote) — phone / browser as a remote overlay over TLS.
