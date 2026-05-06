---
title: Agents & Profiles
order: 4
---

# Agents and Profiles

Three concepts, each with its own TOML block:

| Block | Purpose |
| --- | --- |
| `[agent]` | Global agent defaults (default agent id, default profile). |
| `[[agents]]` | Registry of agent vendors (claude-code, codex, opencode, custom acp). |
| `[[profiles]]` | Registry of presets binding an agent + model + cwd + system prompt + mode. |

## Example

```toml
[agent]
default = "claude-code"
default_profile = "ask"

[[agents]]
id = "claude-code"
provider = "acp-claude-code"
model = "claude-sonnet-4-5"
command = "bunx"
args = ["--bun", "@zed-industries/claude-code-acp"]

[agents.env]
ANTHROPIC_API_KEY = "${env:ANTHROPIC_API_KEY}"

[[profiles]]
id = "ask"
agent = "claude-code"
model = "claude-haiku-4-5"
system_prompt = ["~/.config/hyprpilot/prompts/ask.md"]

[[profiles]]
id = "strict"
agent = "claude-code"
model = "claude-opus-4-5"
system_prompt = [
  "~/.config/hyprpilot/prompts/base.md",
  "~/.config/hyprpilot/prompts/strict.md"
]
```

## Supported providers

`provider` on `[[agents]]` is a closed set:

| Provider | Vendor | Default command |
| --- | --- | --- |
| `acp-claude-code` | Anthropic Claude via [`@zed-industries/claude-code-acp`](https://www.npmjs.com/package/@zed-industries/claude-code-acp) | `bunx --bun @zed-industries/claude-code-acp` |
| `acp-codex` | OpenAI Codex via [`codex-acp`](https://www.npmjs.com/package/@zed-industries/codex-acp) | `bunx --bun @zed-industries/codex-acp` |
| `acp-opencode` | [opencode](https://opencode.ai) | `opencode acp` |
| `acp-custom` | Captain-supplied ACP-speaking binary | (you provide `command` + `args`) |

## Profile fields

| Field | Type | Notes |
| --- | --- | --- |
| `id` | string (required) | Unique within `[[profiles]]`. Cross-referenced by `agent.default_profile`. |
| `agent` | string (required) | Must match an `[[agents]].id`. |
| `model` | string (optional) | Overrides the agent's default model for this profile. |
| `cwd` | path (optional) | Per-profile working directory. Falls back to the daemon's cwd. |
| `mode` | string (optional) | Vendor-specific mode (e.g. claude-code's `plan` / `default`). |
| `system_prompt` | path[] (optional) | Files read + concatenated (blank-line separator) at resolve time. `[]` is the explicit "no prompt" off-switch. |
| `mcps` | path[] (optional) | Per-profile MCP override; replaces the global `mcps` wholesale. `[]` = no MCPs. |

## Cross-field rules

- `agent.default` → must reference a real `[[agents]].id`.
- `agent.default_profile` → must reference a real `[[profiles]].id`.
- `[[profiles]].agent` → must reference a real `[[agents]].id`.

Typos abort startup with a readable error pointing at the offending field.

## Resolution order

When you submit a prompt:

1. The profile you explicitly picked (CLI `--profile <id>`) wins.
2. Otherwise `agent.default_profile`.
3. Otherwise the first `[[profiles]]` matching `agent.default`.
4. Otherwise the first `[[agents]]` entry on its own.

You can spawn multiple instances of the same profile — they run as independent sessions side-by-side.

## System prompt composition

```toml
system_prompt = [
  "~/.config/hyprpilot/prompts/base.md",
  "~/.config/hyprpilot/prompts/strict.md"
]
```

Files are concatenated (with blank-line separators) and prepended to your first prompt — the agent reads the prompt files as context, then your message. Use it to compose a base persona + per-profile addendum without juggling templates.
