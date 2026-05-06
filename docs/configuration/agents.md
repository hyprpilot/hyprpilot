---
title: Agents
order: 3
---

# Agents

An agent is the upstream coding tool you're talking to: claude-code, codex, opencode, or your own. Profiles spawn agents — see [Profiles](./profiles) for the much more interesting half of this.

## Registering an agent

```toml
[[agents]]
id = "claude-code"                 # how profiles reference it
provider = "acp-claude-code"       # which vendor adapter to use
model = "claude-sonnet-4-5"        # default model for instances of this agent

[agents.env]
ANTHROPIC_API_KEY = "${env:ANTHROPIC_API_KEY}"
```

`${env:VAR}` interpolates from your shell environment at daemon start.

## Supported providers

| Provider | Vendor |
| --- | --- |
| `acp-claude-code` | Anthropic Claude via [`@zed-industries/claude-code-acp`](https://www.npmjs.com/package/@zed-industries/claude-code-acp). |
| `acp-codex` | OpenAI Codex via [`@zed-industries/codex-acp`](https://www.npmjs.com/package/@zed-industries/codex-acp). |
| `acp-opencode` | [opencode](https://opencode.ai). |
| `acp-custom` | Any binary that speaks the [Agent Client Protocol](https://agentclientprotocol.com/) — supply your own `command` + `args`. |

## Custom command

By default each provider knows how to spawn its own vendor — you don't need to specify `command` / `args`. Override only when you need a non-default invocation:

```toml
[[agents]]
id = "claude-code-canary"
provider = "acp-claude-code"
command = "bunx"
args = ["--bun", "@zed-industries/claude-code-acp@canary"]
```

## Custom ACP agents

Bring any binary that speaks ACP:

```toml
[[agents]]
id = "my-agent"
provider = "acp-custom"
command = "/usr/local/bin/my-agent"
args = ["--acp"]

[agents.env]
MY_AGENT_TOKEN = "${env:MY_AGENT_TOKEN}"
```

## Choosing the default

```toml
[agent]
default = "claude-code"
default_profile = "ask"
```

`default` falls back when nothing else picks an agent (a profile didn't specify one, no `[[profiles]]` matched). `default_profile` is the profile new instances open with.

Both must reference real `id`s — the daemon refuses to start with a typo.
