---
title: Agents
order: 3
---

# Agents

An agent is the upstream coding tool you're talking to: claude-code, codex, opencode, or your own. Profiles spawn agents — see [Profiles](./profiles) for the much more interesting half of this.

Each agent has two launch surfaces:

- `command` / `args` start the ACP bridge used by the overlay daemon.
- `[agents.spawn]` starts the vendor's native TUI for `hyprpilot spawn`.

## Registering an agent

```toml
[[agents]]
id = "claude-code"                 # how profiles reference it
provider = "acp-claude-code"       # which vendor adapter to use
model = "claude-sonnet-4-5"        # default model for instances of this agent
command = "bunx"                   # ACP bridge command
args = ["--bun", "-y", "@agentclientprotocol/claude-agent-acp@latest"]

[agents.spawn]
command = "claude"                 # native provider TUI command
args = []

[agents.env]
ANTHROPIC_API_KEY = "${env:ANTHROPIC_API_KEY}"
```

`${env:VAR}` interpolates from your shell environment at daemon start.

## Supported providers

| Provider | Vendor |
| --- | --- |
| `acp-claude-code` | Anthropic Claude via [`@agentclientprotocol/claude-agent-acp`](https://www.npmjs.com/package/@agentclientprotocol/claude-agent-acp). |
| `acp-codex` | OpenAI Codex via [`@zed-industries/codex-acp`](https://www.npmjs.com/package/@zed-industries/codex-acp). |
| `acp-opencode` | [opencode](https://opencode.ai). |
| `acp` | Any binary that speaks the [Agent Client Protocol](https://agentclientprotocol.com/) — supply your own `command` + `args`. |

## ACP command

Defaults seed `command` / `args` for the built-in providers. Override them when you need a non-default ACP bridge invocation:

```toml
[[agents]]
id = "claude-code-canary"
provider = "acp-claude-code"
command = "bunx"
args = ["--bun", "-y", "@agentclientprotocol/claude-agent-acp@latest"]
```

## Direct provider TUI command

`[agents.spawn]` is optional for overlay-managed sessions, but required by `hyprpilot spawn`. It is separate from the ACP bridge because the direct command should run the provider's native TUI:

```toml
[[agents]]
id = "codex"
provider = "acp-codex"
command = "bunx"
args = ["--bun", "-y", "@zed-industries/codex-acp@latest"]

[agents.spawn]
command = "codex"
args = []
```

Extra provider arguments can be passed after `--`:

```sh
hyprpilot spawn engineer -- --debug
```

`hyprpilot spawn` still resolves the selected profile first, then projects the profile's model, mode, effort, system prompt, and MCPs onto the provider CLI where supported.

## Custom ACP agents

Bring any binary that speaks ACP:

```toml
[[agents]]
id = "my-agent"
provider = "acp"
command = "/usr/local/bin/my-agent"
args = ["--acp"]

[agents.spawn]
command = "/usr/local/bin/my-agent"
args = ["--tui"]

[agents.env]
MY_AGENT_TOKEN = "${env:MY_AGENT_TOKEN}"
```

## Choosing the default

```toml
[profile]
default = "engineer"
```

`[profile] default` is the profile new overlay-managed instances open with when no profile was selected. It must reference a real `[[profiles]].id` — the daemon refuses to start with a typo.
