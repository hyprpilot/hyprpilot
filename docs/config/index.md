---
title: Config Overview
order: 1
prev: false
---

# {{ $frontmatter.title }}

Everything hyprpilot does is driven by one layered config: which vendors it can launch, which session profiles exist, which MCP servers and skills a launch carries, and how the launch chrome behaves. This section is the whole configuration surface — one page per root section, each with the narrative and the field-by-field reference together.

<!-- more -->

## Formats

Hyprpilot reads **YAML, TOML, or JSON** — the same shape in any of them. The docs write **YAML** throughout (the recommended format, and the most readable for nested profiles); drop a `config.toml` or `config.json` instead if you prefer, the field names are identical.

The global config lives at `~/.config/hyprpilot/config.{toml,json,yaml,yml}`, searched across the four extensions in priority order:

```txt
.toml → .json → .yaml → .yml
```

If two files with different extensions coexist at the same layer (say `config.toml` **and** `config.yaml`), hyprpilot errors at load rather than silently picking one. `--config <path>` infers the format from the extension.

## A complete example

```yaml
# ~/.config/hyprpilot/config.yaml
profile:
  default: engineer

profiles:
  - id: engineer
    agent: claude-code
    model: claude-sonnet-4-5
    cwd: ~/code/my-project
    system_prompt:
      - file: ~/.config/hyprpilot/prompts/engineer.md
    mcps:
      - file: ~/.claude.json

patches:
  - system_prompt:
      - file: ~/.config/hyprpilot/prompts/base.md

multiplexer:
  set_title: true

logging:
  level: info
```

## Root sections

| Section        | Shape               | Purpose                                                                     |
| -------------- | ------------------- | --------------------------------------------------------------------------- |
| `agents`       | list, keyed by `id` | Vendor CLI registry. See [Agents](./agents).                                |
| `profile`      | singleton           | Picks the default session profile. See [Profiles](./profiles).              |
| `profiles`     | list, keyed by `id` | Session presets — at least one is required. See [Profiles](./profiles).     |
| `mcp` / `mcps` | block / list        | Skills channel + MCP catalogue. See [MCP](./mcp).                           |
| `patches`      | list                | Partial-profile overlays, additive across layers. See [Patches](./patches). |
| `multiplexer`  | singleton           | tmux/zellij title rename. See [Multiplexer](./multiplexer).                 |
| `logging`      | singleton           | Tracing filter level. See [Logging](./logging).                             |

There is **no** root-level `system_prompt` / `mcps` / `mcp` / `cwd` field — those are per-profile, or shared via [`patches`](./patches).

## Layers

Config resolves in layers — compiled defaults → global config → named config-layer profile → `patches` / `--with-config` — with later layers overriding earlier ones per field (and `patches` **accumulating** across layers). [Layering](./layering) covers discovery, the merge rules, and validation.

Defaults quoted in the reference tables come from the compiled `src/config/defaults.toml`, the single source of truth (the binary embeds TOML internally — your own config can be any of the three formats):

::: details The compiled defaults, verbatim

<<< @/../src/config/defaults.toml

:::

## Validation

Every section validates types and rejects unknown fields at load — typos fail fast with an error naming the offending field path. Cross-field references are checked too: `profiles[].agent` must reference a real `agents[].id`, and `profile.default` must name a real `profiles[].id`. The `profiles` list must be non-empty — a fresh install with no profile refuses to launch rather than guessing.
