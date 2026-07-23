---
title: Agents
order: 3
---

# Agents

An agent is the vendor CLI hyprpilot launches: `claude`, `codex`, or
`opencode`. Profiles reference agents by id — see
[Profiles](./profiles) for the more interesting half of this. An
`[[agents]]` entry declares the native binary to `exec()` and which
vendor projection to apply.

## Registering an agent

```toml
[[agents]]
id = "claude-code"                 # how profiles reference it
provider = "claude-code"           # closed provider enum
command = "claude"                 # the NATIVE binary hyprpilot execs
args = []                          # bare → the vendor's interactive TUI
model = "claude-sonnet-4-5"        # optional default model
[agents.env]
ANTHROPIC_API_KEY = "${env:ANTHROPIC_API_KEY}"
```

`${env:VAR}` / `${VAR}` and `~` in path- and env-valued fields expand at
launch time from your shell environment.

The compiled defaults already seed the three built-ins — `claude-code`,
`codex`, and `opencode`, each with `args = []` — so most captains never
write an `[[agents]]` block at all. You only add one to point at a
different binary, pin a default model, or set agent-wide env.

## Fields

| Field | Type | What it does |
| --- | --- | --- |
| `id` | string | How profiles reference this agent. |
| `provider` | enum | Which vendor projection to apply (below). |
| `command` | string | The native CLI binary hyprpilot `exec()`s. Mandatory. |
| `args` | string[] | Base arguments. `[]` launches the vendor's interactive TUI. |
| `model` | string (optional) | Default model; a profile's `model` overrides it. |
| `effort` | string (optional) | Default reasoning-effort knob. |
| `cwd` | path (optional) | Default working directory; `--cwd` and a profile `cwd` override it. |
| `env` | table (optional) | Environment overlaid on the inherited shell env. |

## Providers

`provider` is a closed set. Each variant maps to a per-vendor native-CLI
command builder that projects the resolved profile (model, effort, mode,
system prompt, MCP catalog, tool policy) onto that vendor's flags and
environment:

| Provider | Vendor | Default `command` |
| --- | --- | --- |
| `claude-code` | Anthropic Claude Code | `claude` |
| `codex` | OpenAI Codex | `codex` |
| `opencode` | [opencode](https://opencode.ai) | `opencode` |

There is no generic / `custom` provider — every agent must be one of
these three so that every profile gets the full native projection. A
hand-rolled CLI can still be launched by declaring its own `command` /
`args` under one of these providers (accepting that vendor's flag
conventions), or per-profile via the flat
[`command`/`args`/`env` override](./profiles#the-flat-command-args-env-override).

### What each projection does

- **`claude-code`** — projects MCP servers as an inline `--mcp-config`
  JSON blob and tool policy as `--allowedTools` / `--disallowedTools`
  (`mcp__server__tool` naming). Model / system prompt map to the native
  flags.
- **`codex`** — projects MCP servers and overrides as `-c
  mcp_servers.<name>.*` config keys, and tool policy as exact-name
  `enabled_tools` / `disabled_tools` / per-tool `approval_mode`. Codex
  does not support wildcard tool patterns in those fields, so wildcard
  patterns are skipped for Codex with a warning.
- **`opencode`** — projects config via `OPENCODE_CONFIG_CONTENT` and
  tool policy as ordered `OPENCODE_PERMISSION` rules (`server_tool`
  naming, wildcards supported).

MCP transport (stdio / http / sse) is inferred from field presence
(`command` → stdio, `url` → http/sse). Any provider-native argument you
pass after `--` suppresses the generated equivalent, so you can always
override hyprpilot's projection by hand.

## Modes

`mode` on a profile is a free string projected onto each vendor's native
mode surface:

- **claude-code** — e.g. `plan` / `default`.
- **codex** has no single `mode` flag. For Codex profiles, `mode` must
  be either an approval policy (`untrusted`, `on-request`, `never`, or
  the deprecated `on-failure`) mapped to `--ask-for-approval`, or a
  sandbox mode (`read-only`, `workspace-write`, `danger-full-access`)
  mapped to `--sandbox`. An unsupported value fails before the terminal
  is handed to `codex`.

## Choosing the default

```toml
[profile]
default = "engineer"
```

`[profile] default` names the `[[profiles]].id` that bare `hyprpilot`
launches when no `--profile` is passed. It must reference a real profile
— a typo fails validation at load.
