---
title: Profiles
order: 2
---

# Profiles

A profile is a preset that binds together everything a launch needs:
which agent vendor, which model, where it runs, what system prompt it
loads, which MCPs it has access to, what mode it starts in. Pick a
profile — from the interactive picker or `--profile`/`-p` — and hyprpilot
resolves it, projects it onto the vendor's native flags, and `exec()`s.

This is the most important config you'll write. Everything else tunes the
launch chrome — profiles tune the work.

## Anatomy

```toml
[[profiles]]
id = "engineer"                          # picker label, also `-p engineer`
agent = "claude-code"                    # must match an [[agents]].id
model = "claude-sonnet-4-5"              # optional; overrides the agent's default
cwd = "~/code/hyprpilot"                 # optional; falls back to $PWD
mode = "default"                         # optional; vendor-specific (e.g. plan / default)
system_prompt = [
  { file = "~/.config/hyprpilot/prompts/base.md" },
  { file = "~/.config/hyprpilot/prompts/engineer.md" },
]
mcps = [
  { file = "~/.config/hyprpilot/mcps/team.json" },
  { file = "~/.claude.json" },
]
```

Launch it:

```sh
hyprpilot profiles            # list configured profiles
hyprpilot -p engineer         # resolve + exec directly
hyprpilot                     # pick a profile interactively, then exec
```

## Fields

| Field | Type | What it does |
| --- | --- | --- |
| `id` | string | Unique within `[[profiles]]`. The picker row + `-p <id>`. |
| `agent` | string | Which `[[agents]]` entry to launch. |
| `model` | string (optional) | Overrides the agent's default model. Precedence is profile > agent > vendor default. |
| `effort` | string (optional) | Reasoning-effort knob, mapped to the vendor's config surface where supported. |
| `cwd` | path (optional) | Where the agent runs. `~`, `${VAR}` expansion supported. |
| `mode` | string (optional) | Vendor-specific starting mode. See [Agents](./agents). |
| `system_prompt` | `{ file, inject? }[]` (optional) | Prompt files prepended to the first turn. `[]` = no prompt. |
| `mcps` | `{ file, ignore? }[]` (optional) | Per-profile MCP catalog. `[]` = no MCPs. See [MCP & skills](./mcp-and-skills). |
| `mcp` | `[mcp]` block (optional) | Per-profile override of the in-tree MCP / skills block. |
| `command` | string (optional) | Replaces the base agent's `command` for this profile. |
| `args` | string[] (optional) | Replaces the base agent's `args` for this profile. |
| `env` | table (optional) | Overlays the base agent's `env` per-key. |

## Picking the default

```toml
[profile]
default = "engineer"          # which [[profiles]] bare `hyprpilot` picks
```

Resolution at launch time:

1. `--profile <id>` (`-p`) wins.
2. Otherwise `[profile] default`.
3. Otherwise — if you didn't pass a profile and no default is set — the
   interactive picker opens. If neither a picked nor a default profile
   resolves to a real `[[profiles]]` entry, the launch errors. There is
   no bare-agent fallback.

## The flat `command` / `args` / `env` override

Sometimes a profile needs to launch a *different* binary or extra flags
than the base agent entry declares — a canary build, a wrapper script, a
long flag list. Instead of a nested override block, a profile carries
three flat top-level fields:

```toml
[[profiles]]
id = "engineer-canary"
agent = "claude-code"
command = "claude-canary"                 # REPLACES the agent's command
args = ["--dangerously-skip-permissions"] # REPLACES the agent's args
[profiles.env]
ANTHROPIC_LOG = "debug"                    # OVERLAYS the agent's env per-key
```

- **`command`** — when set, replaces the base agent's `command`
  wholesale for this profile.
- **`args`** — when set, replaces the base agent's `args` wholesale.
  Flags have no stable key to merge by (`--flag value`, `-c k=v`,
  positionals), so this is a swap, not an append: to add one flag to an
  otherwise long agent-args list, restate the full list here.
- **`env`** — overlays onto the base agent's `env` per key. The
  profile's key wins on collision; keys the profile doesn't mention are
  left untouched.

The agent's `provider` still drives the native-flag projection (model,
mode, MCP config); the flat override only swaps what binary is launched
with what arguments and environment.

## System prompts

`system_prompt` is an array of `{ file, inject? }` entries. Each file is
read at **resolve** time (not ahead of time), the surviving bodies are
concatenated with blank-line separators, and the result is prepended to
your first turn so the agent reads it as context before your message.

```toml
system_prompt = [
  { file = "~/.config/hyprpilot/prompts/base.md" },        # shared persona
  { file = "~/.config/hyprpilot/prompts/engineer.md" },    # per-profile addendum
]
```

Composition lets a base persona + per-profile addendum land without
juggling templates. `system_prompt = []` is the explicit "no prompt"
off-switch. Because files are read at resolve time, a missing file fails
loudly on the next launch rather than silently.

### Per-entry inject toggle

Each entry takes an optional `inject` object. On the launcher path only
the fresh-launch injection runs, so the relevant gate is `on_create`
(default `true`):

```toml
system_prompt = [
  { file = "~/.config/hyprpilot/prompts/base.md" },                       # injects (on_create default true)
  { file = "~/.config/hyprpilot/prompts/notes.md", inject = { on_create = false } },  # skipped
]
```

## MCPs and skills

MCP servers extend an agent with tools; skills attach markdown context.
Both are documented on their own page — see
[MCP & skills](./mcp-and-skills). In short:

- `mcps = [ … ]` on a profile is a per-profile MCP catalog that
  wholesale-replaces the shared set; `mcps = []` means "no MCPs".
- The in-tree `hyprpilot` MCP server (which delivers your skills) is
  configured under a `[mcp]` block, seeded globally via `[[patches]]`
  and overridable per-profile with `[profiles.<id>.mcp]`.

## Examples

### A planning profile with no MCPs

```toml
[[profiles]]
id = "plan"
agent = "claude-code"
model = "claude-opus-4-5"
mode = "plan"
mcps = []
```

### A code-review profile pinned to a repo

```toml
[[profiles]]
id = "review-hyprpilot"
agent = "claude-code"
cwd = "~/code/hyprpilot"
system_prompt = [{ file = "~/.config/hyprpilot/prompts/reviewer.md" }]
```

### A profile launching a wrapper binary

```toml
[[profiles]]
id = "sandboxed"
agent = "claude-code"
command = "firejail"
args = ["--net=none", "claude"]
```
