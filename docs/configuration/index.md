---
title: Configuration overview
order: 1
---

# Configuration

Hyprpilot reads layered config. Each source overrides the one before it,
so you only write what you want to change:

1. **Compiled defaults** — every knob has a working default, baked into
   the binary (`src/config/defaults.toml`).
2. **Global config** — `~/.config/hyprpilot/config.{toml,json,yaml,yml}`,
   or `--config <path>`.
3. **Named config-layer profile** —
   `~/.config/hyprpilot/profiles/<name>.{ext}`, picked with
   `--config-profile <name>` or `HYPRPILOT_CONFIG_PROFILE=<name>`.
4. **`[[patches]]` and `--with-config`** — profile overlays applied at
   resolve time. See [Patches & overlays](./patches).

The global config and named profiles are searched across four
extensions in priority order (`.toml` → `.json` → `.yaml` → `.yml`);
two coexisting files at the same layer error at load. `--config <path>`
infers the format from the extension.

A minimal config that launches only needs one `[[agents]]` entry (the
defaults already seed `claude-code`, `codex`, and `opencode`) plus one
`[[profiles]]` entry:

```toml
[[profiles]]
id = "engineer"
agent = "claude-code"          # references a seeded [[agents]] id
model = "claude-sonnet-4-5"    # optional

[profile]
default = "engineer"           # which profile bare `hyprpilot` picks
```

## Two `profile` concepts

The word "profile" lives in two parallel namespaces:

| Concept | Where | Purpose |
| --- | --- | --- |
| Config-layer profile | `--config-profile` flag / `HYPRPILOT_CONFIG_PROFILE` | Pick a different config TOML overlay (e.g. `work` vs `personal`). |
| Session profile | `[[profiles]]` blocks in config | Pick which agent + model + cwd + system prompt + MCPs a launch uses, addressed with `--profile`/`-p`. |

This page documents the layering. For session profiles — by far the most
important config you'll write — see [Profiles](./profiles).

## Where things live

| Path | What |
| --- | --- |
| `~/.config/hyprpilot/config.{toml,json,yaml,yml}` | Global config. |
| `~/.config/hyprpilot/profiles/*.{ext}` | Named config-layer overlays. |
| `~/.config/hyprpilot/skills/<slug>/SKILL.md` | Skill bundles (default catalog root). |
| `~/.config/hyprpilot/mcps/*.json` | MCP catalog files (your convention). |

`~` and `${VAR}` / `${env:VAR}` in path-valued fields expand at consume
time; relative paths resolve against the current directory.

## Root sections

The config root carries a small number of top-level sections:

```toml
[logging]
level = "info"           # trace | debug | info | warn | error

[multiplexer]
set_title = true         # tmux/zellij window rename (default on)
```

- **`[logging] level`** — the tracing filter applied when neither
  `--log-level` nor `RUST_LOG` is set. Precedence:
  `--log-level` → `RUST_LOG` → `[logging] level` → the built-in default.
- **`[multiplexer] set_title`** — rename the current tmux window /
  zellij tab to `hyprpilot@<cwd>` before `exec()`. See
  [Integration](../guide/integration#multiplexer-window-titles).

Everything else lives under `[[agents]]`, `[[profiles]]`, `[profile]`,
and `[[patches]]`. There is **no** root-level `system_prompt` / `mcps` /
`mcp` / `cwd` field — those are per-profile or shared via
[`[[patches]]`](./patches).

## Validation

Every section validates types and rejects unknown fields at load — typos
fail fast with an error naming the offending field. Cross-field
references are checked too: `[[profiles]].agent` must reference a real
`[[agents]].id`, and `[profile] default` must name a real
`[[profiles]].id`. The `[[profiles]]` list must be non-empty — a fresh
install with no profile refuses to launch rather than guessing.

## Pages in this section

- [Profiles](./profiles) — the heart of the config: agent, model, cwd,
  system prompts, MCPs, and the flat `command`/`args`/`env` override.
- [Agents](./agents) — registering the vendor CLIs
  (claude-code, codex, opencode).
- [MCP & skills](./mcp-and-skills) — the MCP catalog, tool policy, and
  the in-tree MCP server that delivers your skills.
- [Patches & overlays](./patches) — `[[patches]]` and `--with-config`
  for profile-shared and per-invocation overrides.
