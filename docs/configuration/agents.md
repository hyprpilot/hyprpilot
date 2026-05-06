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

Each provider is wired in `match_provider_agent()` in `src-tauri/src/adapters/acp/agents/`. Adding a new vendor is one trait impl + one match arm.

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

Garde validates these at startup. Typos abort the boot with a readable message.

## Resolution order

When you submit a prompt:

1. Explicit `--profile <id>` (CLI) or `profile_id` (RPC) — wins.
2. `agent.default_profile` (if both `agent` and `default_profile` are set).
3. First `[[profiles]]` entry matching `agent.default`.
4. First `[[agents]]` entry (no profile, vendor defaults).

Multiple instances of the same `(agent, profile)` are addressable by distinct UUIDs — N twins of one profile are first-class.

## System prompt composition

```toml
system_prompt = [
  "~/.config/hyprpilot/prompts/base.md",
  "~/.config/hyprpilot/prompts/strict.md"
]
```

Files are read at resolve time and concatenated with blank-line separators. No external preprocessor; just a plain file array. Empty string (`""`) entries are skipped silently; missing files abort the spawn.

Per-vendor injection strategy:

- **claude-code** — prepended to the first prompt's text block (no spawn-time hook in the SDK).
- **codex** — passed via `-c instructions=…` at spawn time.
- **opencode** — prepended to the first prompt (same as claude-code).

Captain doesn't see this distinction; the agent just gets the concatenated prompt at the right point.
