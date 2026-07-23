---
title: '[[agents]]'
order: 10
---

# {{ $frontmatter.title }}

The vendor registry. Each entry declares a native CLI hyprpilot can `exec()` and which per-vendor projection to apply. Narrative: [Features → Agents](../features/agents).

<!-- more -->

```toml
[[agents]]
id = "claude-code"
provider = "claude-code"
command = "claude"
args = []
model = "claude-sonnet-4-5"

[agents.env]
ANTHROPIC_API_KEY = "${env:ANTHROPIC_API_KEY}"
```

## Fields

| Field      | Type              | Default | What it does                                                                         |
| ---------- | ----------------- | ------- | ------------------------------------------------------------------------------------ |
| `id`       | string            | —       | How profiles reference this agent. Unique within `[[agents]]`.                       |
| `provider` | enum              | —       | Which vendor projection to apply: `claude-code`, `codex`, or `opencode`. Closed set. |
| `command`  | string            | —       | The native CLI binary hyprpilot `exec()`s. Mandatory.                                |
| `args`     | string[]          | `[]`    | Base arguments. `[]` launches the vendor's interactive TUI.                          |
| `model`    | string (optional) | unset   | Default model; a profile's `model` overrides it (profile > agent > vendor default).  |
| `effort`   | string (optional) | unset   | Default reasoning-effort knob, mapped to the vendor where supported.                 |
| `cwd`      | path (optional)   | unset   | Default working directory; a profile `cwd` and `--cwd` override it.                  |
| `env`      | table (optional)  | `{}`    | Environment overlaid on the inherited shell env.                                     |

`${env:VAR}` / `${VAR}` and `~` expand at launch time from your shell environment.

## Seeded entries

The compiled defaults seed three entries — override one by redeclaring its `id` (whole-entry replace, no field-level merge):

| `id`          | `provider`    | `command`  | `args` |
| ------------- | ------------- | ---------- | ------ |
| `claude-code` | `claude-code` | `claude`   | `[]`   |
| `codex`       | `codex`       | `codex`    | `[]`   |
| `opencode`    | `opencode`    | `opencode` | `[]`   |

There is no `[agent]` singleton and no generic/custom provider variant — see [Features → Agents](../features/agents#the-provider-enum) for how to launch wrapper binaries anyway.
