---
title: '[profile] & [[profiles]]'
order: 20
---

# {{ $frontmatter.title }}

Session presets and the default pick. Narrative: [Features → Profiles](../features/profiles).

<!-- more -->

## `[profile]`

The singleton that picks the default session profile:

```toml
[profile]
default = "engineer"
```

| Field     | Type              | Default | What it does                                                                                    |
| --------- | ----------------- | ------- | ----------------------------------------------------------------------------------------------- |
| `default` | string (optional) | unset   | The `[[profiles]].id` bare `hyprpilot` launches when `-p` is omitted. Must name a real profile. |

When unset and `-p` is omitted, the interactive picker opens.

## `[[profiles]]`

```toml
[[profiles]]
id = "engineer"
agent = "claude-code"
model = "claude-sonnet-4-5"
cwd = "~/code/hyprpilot"
mode = "default"
system_prompt = [{ file = "~/.config/hyprpilot/prompts/base.md" }]
mcps = [{ file = "~/.claude.json" }]
```

The list must be **non-empty** — the compiled defaults seed zero profiles, and validation rejects an empty list at load.

| Field           | Type                             | Default | What it does                                                                                                  |
| --------------- | -------------------------------- | ------- | ------------------------------------------------------------------------------------------------------------- |
| `id`            | string                           | —       | Unique within `[[profiles]]`. The picker row + `-p <id>`.                                                     |
| `agent`         | string                           | —       | Which `[[agents]]` entry to launch. Must reference a real `[[agents]].id`.                                    |
| `model`         | string (optional)                | unset   | Overrides the agent's default model. Precedence: profile > agent > vendor default.                            |
| `effort`        | string (optional)                | unset   | Reasoning-effort knob, mapped to the vendor's config surface where supported.                                 |
| `cwd`           | path (optional)                  | unset   | Where the agent runs. `~`, `${VAR}` expansion supported; falls back to the agent `cwd`, then `$PWD`.          |
| `mode`          | string (optional)                | unset   | Vendor-specific starting mode. See [Features → Agents → Modes](../features/agents#modes).                     |
| `system_prompt` | `{ file, inject? }[]` (optional) | unset   | Prompt files read at resolve time and prepended to the first turn. `[]` = no prompt.                          |
| `mcps`          | `{ file … }[]` (optional)        | unset   | Per-profile MCP catalogue — wholesale-replaces the shared set. `[]` = no MCPs. See [`[mcp]` & `mcps`](./mcp). |
| `mcp`           | `[mcp]` block (optional)         | unset   | Per-profile override of the in-tree MCP / skills block — wholesale-replaces the global.                       |
| `command`       | string (optional)                | unset   | Replaces the base agent's `command` wholesale for this profile.                                               |
| `args`          | string[] (optional)              | unset   | Replaces the base agent's `args` wholesale for this profile.                                                  |
| `env`           | table (optional)                 | `{}`    | Overlays the base agent's `env` per key; the profile's key wins on collision.                                 |

### `system_prompt` entries

| Field    | Type              | Default | What it does                                                                         |
| -------- | ----------------- | ------- | ------------------------------------------------------------------------------------ |
| `file`   | path              | —       | Prompt file, read at resolve time — a missing file fails the launch.                 |
| `inject` | object (optional) | unset   | Injection gates. On the launcher path only `on_create` (default `true`) is relevant. |
