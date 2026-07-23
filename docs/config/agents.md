---
title: Agents
order: 20
---

# {{ $frontmatter.title }}

An agent is the vendor CLI hyprpilot launches: `claude`, `codex`, or `opencode`. An `agents` entry declares the native binary to `exec()` and which vendor projection to apply; profiles reference agents by id.

<!-- more -->

## Registering an agent

The compiled defaults already seed the three built-ins — `claude-code`, `codex`, and `opencode`, each with `args: []` — so most captains never write an `agents` entry at all. You only add one to point at a different binary, pin a default model, or set agent-wide env:

```yaml
agents:
  - id: claude-code # how profiles reference it
    provider: claude-code # closed provider enum
    command: claude # the NATIVE binary hyprpilot execs
    args: [] # bare → the vendor's interactive TUI
    model: claude-sonnet-4-5 # optional default model
    env:
      ANTHROPIC_API_KEY: ${env:ANTHROPIC_API_KEY}
```

`${env:VAR}` / `${VAR}` and `~` in path- and env-valued fields expand at launch time from your shell environment.

## Fields

| Field      | Type              | Default | What it does                                                                         |
| ---------- | ----------------- | ------- | ------------------------------------------------------------------------------------ |
| `id`       | string            | —       | How profiles reference this agent. Unique within `agents`.                           |
| `provider` | enum              | —       | Which vendor projection to apply: `claude-code`, `codex`, or `opencode`. Closed set. |
| `command`  | string            | —       | The native CLI binary hyprpilot `exec()`s. Mandatory.                                |
| `args`     | string[]          | `[]`    | Base arguments. `[]` launches the vendor's interactive TUI.                          |
| `model`    | string (optional) | unset   | Default model; a profile's `model` overrides it (profile > agent > vendor default).  |
| `effort`   | string (optional) | unset   | Default reasoning-effort knob, mapped to the vendor where supported.                 |
| `cwd`      | path (optional)   | unset   | Default working directory; a profile `cwd` and `--cwd` override it.                  |
| `env`      | map (optional)    | `{}`    | Environment overlaid on the inherited shell env.                                     |

## Seeded entries

The compiled defaults seed three entries — override one by redeclaring its `id` (whole-entry replace, no field-level merge):

| `id`          | `provider`    | `command`  | `args` |
| ------------- | ------------- | ---------- | ------ |
| `claude-code` | `claude-code` | `claude`   | `[]`   |
| `codex`       | `codex`       | `codex`    | `[]`   |
| `opencode`    | `opencode`    | `opencode` | `[]`   |

There is no `agent` singleton and no generic/custom provider variant — see [the provider enum](#the-provider-enum) for how to launch wrapper binaries anyway.

## The provider enum

`provider` is a closed set — every agent must be one of the three variants so that every profile gets the full native projection:

| Provider      | Vendor                          | Default `command` |
| ------------- | ------------------------------- | ----------------- |
| `claude-code` | Anthropic Claude Code           | `claude`          |
| `codex`       | OpenAI Codex                    | `codex`           |
| `opencode`    | [opencode](https://opencode.ai) | `opencode`        |

There is no generic escape-hatch provider. If you want to launch a wrapper or a hand-rolled CLI, declare its `command` / `args` under one of these providers (accepting that vendor's flag conventions), or swap the binary per-profile via the flat [`command`/`args`/`env` override](./profiles#the-flat-command-args-env-override).

## Native-flag projection

Each provider variant maps to a per-vendor command builder that projects the resolved profile — model, effort, mode, system prompt, MCP catalogue, tool policy — onto that vendor's flags and environment:

- **`claude-code`** — `--model`, `--effort`, `--permission-mode` (from `mode`), `--append-system-prompt`, MCP servers as `--mcp-config <path>` pointing at a per-launch 0600 temp file (keeps expanded header secrets out of the world-readable argv — see [MCP → Secrets](./mcp#secrets-in-the-vendor-handoff)), and tool policy as `--allowedTools` / `--disallowedTools` (`mcp__server__tool` naming).
- **`codex`** — `--model`, effort as a `-c model_reasoning_effort=…` override, MCP servers as `-c mcp_servers.<name>.*` config keys, and tool policy as exact-name `enabled_tools` / `disabled_tools` / per-tool `approval_mode`. Codex does not support wildcard tool patterns in those fields, so wildcard patterns are skipped for Codex with a warning.
- **`opencode`** — `--model`, `mode` as the opencode `--agent` name (a synthetic `hyprpilot` agent when unset), config (system prompt, effort variant, MCP servers) via `OPENCODE_CONFIG_CONTENT`, and tool policy as ordered `OPENCODE_PERMISSION` rules (`server_tool` naming, wildcards supported).

MCP transport (stdio / http / sse) is inferred from field presence (`command` → stdio, `url` → http/sse). Any provider-native argument you pass after `--` suppresses the generated equivalent, so you can always override hyprpilot's projection by hand.

## Modes

`mode` on a profile (or `--mode` on the CLI) is a free string projected onto each vendor's native mode surface:

- **claude-code** — passed to `--permission-mode` (e.g. `plan`, `default`).
- **codex** — Codex has no single mode flag. The value must be either an approval policy (`untrusted`, `on-request`, `never`, or the deprecated `on-failure`) mapped to `--ask-for-approval`, or a sandbox mode (`read-only`, `workspace-write`, `danger-full-access`) mapped to `--sandbox`. An unsupported value fails before the terminal is handed to `codex`.
- **opencode** — used as the `--agent` name.

## Swapping the agent per launch

The profile is the single source of truth for which agent it runs — there is no `--agent` launch flag. To run an existing profile against a different vendor for one launch, overlay the `agent` field with [`--with-config`](../runtime/with-config):

```sh
hyprpilot engineer --with-config '@{"agent":"codex"}'
```

The overlay wins over whatever agent the (patched) profile names — the profile's own `model` / `mode` / prompt overlays still apply, projected through the new agent's provider. To make the swap permanent, add a dedicated `profiles` entry instead.
