---
title: Agents
order: 20
---

# {{ $frontmatter.title }}

An agent is the vendor CLI hyprpilot launches: `claude`, `codex`, `opencode`, or `amp`. An `agents` entry declares the native binary to `exec()` and which vendor projection to apply; profiles reference agents by id.

<!-- more -->

## Registering an agent

The compiled defaults already seed the four built-ins — `claude-code`, `codex`, `opencode`, and `amp`, each with `args: []` — so most captains never write an `agents` entry at all. You only add one to point at a different binary, pin a default model, or set agent-wide env:

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

| Field      | Type              | Default | What it does                                                                                |
| ---------- | ----------------- | ------- | ------------------------------------------------------------------------------------------- |
| `id`       | string            | —       | How profiles reference this agent. Unique within `agents`.                                  |
| `provider` | enum              | —       | Which vendor projection to apply: `claude-code`, `codex`, `opencode`, or `amp`. Closed set. |
| `command`  | string            | —       | The native CLI binary hyprpilot `exec()`s. Mandatory.                                       |
| `args`     | string[]          | `[]`    | Base arguments. `[]` launches the vendor's interactive TUI.                                 |
| `model`    | string (optional) | unset   | Default model; a profile's `model` overrides it (profile > agent > vendor default).         |
| `effort`   | string (optional) | unset   | Default reasoning-effort knob, mapped to the vendor where supported.                        |
| `cwd`      | path (optional)   | unset   | Default working directory; a profile `cwd` and `--cwd` override it.                         |
| `env`      | map (optional)    | `{}`    | Environment overlaid on the inherited shell env.                                            |

## Seeded entries

The compiled defaults seed four entries — override one by redeclaring its `id` (whole-entry replace, no field-level merge):

| `id`          | `provider`    | `command`  | `args` |
| ------------- | ------------- | ---------- | ------ |
| `claude-code` | `claude-code` | `claude`   | `[]`   |
| `codex`       | `codex`       | `codex`    | `[]`   |
| `opencode`    | `opencode`    | `opencode` | `[]`   |
| `amp`         | `amp`         | `amp`      | `[]`   |

There is no `agent` singleton and no generic/custom provider variant — see [the provider enum](#the-provider-enum) for how to launch wrapper binaries anyway.

## The provider enum

`provider` is a closed set — every agent must be one of these variants so that every profile gets the full native projection:

| Provider      | Vendor                          | Default `command` |
| ------------- | ------------------------------- | ----------------- |
| `claude-code` | Anthropic Claude Code           | `claude`          |
| `codex`       | OpenAI Codex                    | `codex`           |
| `opencode`    | [opencode](https://opencode.ai) | `opencode`        |
| `amp`         | [Amp](https://ampcode.com)      | `amp`             |

There is no generic escape-hatch provider. If you want to launch a wrapper or a hand-rolled CLI, declare its `command` / `args` under one of these providers (accepting that vendor's flag conventions), or swap the binary per-profile via the flat [`command`/`args`/`env` override](./profiles#the-flat-command-args-env-override).

## Native-flag projection

Each provider variant maps to a per-vendor command builder that projects the resolved profile — model, effort, mode, system prompt, MCP catalogue, tool policy — onto that vendor's flags and environment:

- **`claude-code`** — `--model`, `--effort`, `--permission-mode` (from `mode`), `--append-system-prompt`, MCP servers as `--mcp-config <path>` pointing at a per-launch 0600 temp file (keeps expanded header secrets out of the world-readable argv — see [MCP → Secrets](./mcp#secrets-in-the-vendor-handoff)), and tool policy as `--allowedTools` / `--disallowedTools` (`mcp__server__tool` naming).
- **`codex`** — `--model`, effort as a `-c model_reasoning_effort=…` override, MCP servers as `-c mcp_servers.<name>.*` config keys, and tool policy as exact-name `enabled_tools` / `disabled_tools` / per-tool `approval_mode`. Codex does not support wildcard tool patterns in those fields, so wildcard patterns are skipped for Codex with a warning.
- **`opencode`** (V2) — always `--standalone`, and everything else as native config in `OPENCODE_CONFIG_CONTENT`: the system prompt and model on the agent `mode` names (a synthetic `hyprpilot` agent when unset), that agent as `default_agent`, effort joined onto the model as `provider/model#effort`, MCP servers under `mcp.servers`, and tool policy as ordered `permissions` rules (`server_tool` actions, wildcards supported). The TUI takes no `--model` / `--agent`, so neither is ever emitted. `--standalone` gives the launch a private server that inherits its env; without it opencode attaches to one shared background service started with some earlier client's env, and the profile's config, database and API keys are ignored. A `--server` you pass after `--` suppresses it. The model must be a `provider/model` reference; a bare model id fails the launch, since opencode would otherwise drop the whole agent, system prompt included, without a word. An `OPENCODE_CONFIG_CONTENT` you set in the agent's `env` replaces the generated config wholesale — model, agent, system prompt, MCP servers and tool permissions are then not projected, and a warning says so.

- **`amp`** — `--mode` (from `mode`), and MCP servers as `--mcp-config <path>`, a per-launch 0600 temp file like claude's. Amp selects its model and system prompt through the mode and takes no tool policy on its command line, so `model`, `effort`, `system_prompt` and `includeTools` / `excludeTools` / `autoRejectTools` are ignored with a warning: pick the model with `mode`, put instructions in `AGENTS.md`, and disable tools with `amp.tools.disable` in the settings file. Headless launches run `amp --execute` with the prompt on stdin; harness turns add `--stream-json` and `--no-archive-after-execute`, since Amp refuses to continue an archived thread.

MCP transport (stdio / http / sse) is inferred from field presence (`command` → stdio, `url` → http/sse). Any provider-native argument you pass after `--` suppresses the generated equivalent, so you can always override hyprpilot's projection by hand.

## Modes

`mode` on a profile (or `--mode` on the CLI) is a free string projected onto each vendor's native mode surface:

- **claude-code** — passed to `--permission-mode` (e.g. `plan`, `default`).
- **codex** — Codex has no single mode flag. The value must be either an approval policy (`untrusted`, `on-request`, `never`, or the deprecated `on-failure`) mapped to `--ask-for-approval`, or a sandbox mode (`read-only`, `workspace-write`, `danger-full-access`) mapped to `--sandbox`. An unsupported value fails before the terminal is handed to `codex`.
- **opencode** — the agent name, set as `default_agent`.
- **amp** — passed to `--mode` verbatim: `low`, `medium`, `high`, `ultra`, `claude-code`, or a plugin mode's key.

## Separating accounts

Every vendor keeps its account and settings somewhere an environment variable can move, so one profile per account is an `env` patch — claude reads `CLAUDE_CONFIG_DIR`, codex `CODEX_HOME`, opencode `OPENCODE_CONFIG_DIR`, and amp takes its token from `AMP_API_KEY` and its settings file (MCP servers, skill paths, permissions) from `AMP_SETTINGS_FILE`:

```yaml
patches:
  - $match:
      profile: 'amp/*'
    env:
      AMP_API_KEY: ${AMP_API_KEY_WORK}
      AMP_SETTINGS_FILE: ~/.config/amp-work/settings.json

profiles:
  - id: amp/high
    agent: amp
    mode: high
```

`AMP_API_KEY` takes precedence over the account `amp login` saved, so two profiles run side by side under different accounts. Forward `--no-tui` to start an Amp runner from a profile — `hyprpilot amp/high -- --no-tui` — and the threads it serves get the profile's env and MCP servers.

## Swapping the agent per launch

The profile is the single source of truth for which agent it runs — there is no `--agent` launch flag. To run an existing profile against a different vendor for one launch, overlay the `agent` field with [`--with-config`](../runtime/with-config):

```sh
hyprpilot engineer --with-config '@{"agent":"codex"}'
```

The overlay wins over whatever agent the (patched) profile names — the profile's own `model` / `mode` / prompt overlays still apply, projected through the new agent's provider. To make the swap permanent, add a dedicated `profiles` entry instead.
