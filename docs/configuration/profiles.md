---
title: Profiles
order: 2
---

# Profiles

A profile is a preset that binds together everything an agent instance needs: which agent vendor, which model, where it runs, what system prompt it loads, which MCPs it has access to, what mode it starts in. Pick a profile from the palette and a fresh instance spawns ready to work.

This is the most important config you'll write. Everything else (themes, window mode) tunes the chrome — profiles tune the work.

## Anatomy

```toml
[[profiles]]
id = "engineer"                          # palette label, also addressable via CLI
agent = "claude-code"                    # must match an [[agents]].id
model = "claude-sonnet-4-5"              # optional; overrides the agent's default
cwd = "~/code/hyprpilot"                 # optional; falls back to where the daemon was started
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

Spawn it from the palette: `Ctrl+K → profiles → engineer`. Or explicitly from the CLI:

```sh
hyprpilot ctl prompts send --profile engineer "show me the failing tests"
```

You can have multiple instances of the same profile running side-by-side — each gets its own UUID and its own session.

## Fields

| Field | Type | What it does |
| --- | --- | --- |
| `id` | string | Unique within `[[profiles]]`. Shows up as the palette row. |
| `agent` | string | Which `[[agents]]` entry to spawn. |
| `model` | string (optional) | Overrides the agent's default model for this profile. |
| `cwd` | path (optional) | Where the agent operates. `~`, `${VAR}` expansion supported. |
| `mode` | string (optional) | Vendor-specific starting mode. claude-code: `plan` / `default`. codex: approval modes. |
| `system_prompt` | `{ file, inject? }[]` (optional) | Per-entry prompt files prepended to your first prompt. Each entry's `inject.on_create` / `inject.on_update` toggles which bootstrap paths it rides on. `[]` = no prompt. |
| `mcps` | `{ file, ignore? }[]` (optional) | MCP catalog override for this profile. `[]` = no MCPs. |

## Picking the default

```toml
[agent]
default = "claude-code"      # which [[agents]] entry wins when nothing's specified

[profile]
default = "engineer"         # which [[profiles]] new instances pick by default
```

Resolution when you submit a prompt:

1. The profile you picked from the palette (or `--profile <id>` from the CLI) wins.
2. Otherwise `[profile] default`.
3. Otherwise the first `[[profiles]]` matching `[agent] default`.
4. Otherwise the first `[[agents]]` entry by itself.

## System prompts

`system_prompt` is an array of `{ file, inject? }` entries. The daemon reads each file, concatenates the surviving bodies with blank-line separators, and prepends the result to your first prompt. The agent treats it as context, then reads your message.

```toml
system_prompt = [
  { file = "~/.config/hyprpilot/prompts/base.md" },        # shared persona
  { file = "~/.config/hyprpilot/prompts/engineer.md" },    # per-profile addendum
]
```

Composition lets a base persona + per-profile addendum land without juggling templates. `system_prompt = []` is the explicit "no prompt" off-switch.

### Per-entry inject toggles

Each entry takes an optional `inject` object that gates which bootstrap paths the file rides on:

```toml
system_prompt = [
  # Default: rides only on fresh sessions, not on resume.
  { file = "~/.config/hyprpilot/prompts/base.md" },

  # Explicit: rides on both fresh AND resume.
  { file = "~/.config/hyprpilot/prompts/strict.md",
    inject = { on_create = true, on_update = true } },
]
```

| Field | Default | What it gates |
| --- | --- | --- |
| `inject.on_create` | `true` | Whether the file rides when the daemon spawns a fresh session. |
| `inject.on_update` | `false` | Whether the file rides when resuming a persisted session. Default off because the resumed session already carries its own transcript context — re-injecting the prompt on top is usually redundant noise. |

When at least one entry actually injects, the chat shows a `system prompt · <files>` chapter-break banner so you can see what rode along.

## MCPs

MCPs (Model Context Protocol servers) extend an agent with tools — filesystem, search, GitHub, custom services. Each `[[mcps]]` entry points at a JSON file using the standard `mcpServers` shape that Claude Code, Codex, and Cursor all read.

**You can drop your existing `~/.claude.json` straight in.** It works.

```toml
[[mcps]]
file = "~/.claude.json"

[[mcps]]
file = "~/.config/hyprpilot/mcps/team.json"
ignore = ["scratch-*", "*-internal"]
```

Inside each file:

```json
{
  "mcpServers": {
    "filesystem": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
    },
    "github": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-github"],
      "env": {
        "GITHUB_PERSONAL_ACCESS_TOKEN": "${env:GITHUB_TOKEN}"
      }
    }
  }
}
```

Files iterate in order. Same-named servers in later files override earlier ones.

### Ignoring servers

`ignore` is an optional glob array per `[[mcps]]` entry. Server names matching any pattern are dropped before they reach the agent. Globs anchor against the full server name — `work-*` matches `work-foo` but not `pre-work-foo`. Same matcher used for the `autoAcceptTools` / `autoRejectTools` per-server fields.

### Auto-accept / auto-reject

Each server entry takes an optional `hyprpilot` block to short-circuit specific tool calls without surfacing a permission prompt:

```json
{
  "mcpServers": {
    "filesystem": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
      "hyprpilot": {
        "autoAcceptTools": ["read_*"],
        "autoRejectTools": ["delete_*"]
      }
    }
  }
}
```

Globs are server-relative — write `read_*`, not `mcp__filesystem__read_*`. Reject wins over accept when both match.

### Per-profile MCP override

```toml
[[profiles]]
id = "engineer"
agent = "claude-code"
mcps = [
  { file = "~/.config/hyprpilot/mcps/work.json" },
  { file = "~/.claude.json", ignore = ["scratch-*"] },
]
```

`[[profiles]] mcps` wholesale-replaces the global `[[mcps]]` set for that profile. `mcps = []` means "no MCPs at all" — handy for a sandboxed read-only profile.

## Skills

Skills aren't picked at config time — they live in their own catalog and ride along with prompts you specifically attach them to. Configure the catalog roots once:

```toml
[[skills]]
dir = "~/.config/hyprpilot/skills"

[[skills]]
dir = "~/.team/shared-skills"
ignore = ["work-*", "*-experimental"]
```

Each `dir` is a flat directory of `<slug>/SKILL.md` bundles, compatible with [Anthropic's claude-code skill convention](https://github.com/anthropics/claude-code/blob/main/skills/README.md):

```
~/.config/hyprpilot/skills/
├── git-commit/
│   └── SKILL.md
├── linear-issue/
│   ├── SKILL.md
│   └── references/
└── github-pr/
    └── SKILL.md
```

`ignore` is the same glob shape as `[[mcps]]` — slugs matching any pattern are skipped at load time.

In the composer, `Ctrl+K → skills → <slug>` (or type `#<slug>` directly) attaches a skill to your next prompt. The agent reads the skill body first, then your message. Reload after editing a `SKILL.md` via the palette's `skills → reload` action.

### Per-profile skills override

```toml
[[profiles]]
id = "engineer"
agent = "claude-code"
skills = [
  { dir = "~/.config/hyprpilot/skills" },
  { dir = "~/.engineering/skills", ignore = ["draft-*"] },
]
```

Same shape as the root array; wholesale-replaces the global `[[skills]]` for that profile. `skills = []` disables skill loading for the profile.

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

### A read-only filesystem-only profile

```toml
[[profiles]]
id = "browse"
agent = "claude-code"
mcps = [{ file = "~/.config/hyprpilot/mcps/readonly-fs.json" }]
```

…where `readonly-fs.json` lists `filesystem` with `autoRejectTools = ["write_*", "delete_*", "edit_*"]`.
