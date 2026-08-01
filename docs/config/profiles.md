---
title: Profiles
order: 30
---

# {{ $frontmatter.title }}

A profile is a preset that binds together everything a launch needs: which agent vendor, which model, where it runs, what system prompt it loads, which MCPs it has access to, what mode it starts in. Pick a profile — from the interactive picker or the positional `[PROFILE]` argument — and hyprpilot resolves it, projects it onto the vendor's native flags, and `exec()`s.

<!-- more -->

This is the most important config you'll write. Everything else tunes the launch chrome — profiles tune the work.

## Anatomy

```yaml
profiles:
  - id: engineer # picker label, also `hyprpilot engineer`
    agent: claude-code # must match an agents id
    model: claude-sonnet-4-5 # optional; overrides the agent's default
    cwd: ~/code/hyprpilot # optional; falls back to $PWD
    mode: default # optional; vendor-specific (e.g. plan / default)
    system_prompt:
      - file: ~/.config/hyprpilot/prompts/base.md
      - file: ~/.config/hyprpilot/prompts/engineer.md
    mcps:
      - file: ~/.config/hyprpilot/mcps/team.json
      - file: ~/.claude.json
```

Launch it:

```sh
hyprpilot profiles            # list configured profiles
hyprpilot engineer            # resolve + exec directly (positional)
hyprpilot                     # pick a profile interactively, then exec
```

## Picking the default

```yaml
profile:
  default: engineer # which profiles entry bare `hyprpilot` picks
```

Resolution at launch time:

1. The positional `[PROFILE]` id wins.
2. Otherwise `profile.default`.
3. Otherwise — if you didn't pass a profile and no default is set — the interactive picker opens with `profile.default` pre-selected under the cursor. If neither a picked nor a default profile resolves to a real `profiles` entry, the launch errors. There is no bare-agent fallback.

The `profiles` list must be **non-empty** — the compiled defaults seed zero profiles, and validation rejects an empty list at load.

## Fields

### `profile`

| Field     | Type              | Default | What it does                                                                                                 |
| --------- | ----------------- | ------- | ------------------------------------------------------------------------------------------------------------ |
| `default` | string (optional) | unset   | The `profiles[].id` bare `hyprpilot` launches when no positional profile is given. Must name a real profile. |

### `profiles` entries

| Field           | Type                             | Default | What it does                                                                                                                  |
| --------------- | -------------------------------- | ------- | ----------------------------------------------------------------------------------------------------------------------------- |
| `id`            | string                           | —       | Unique within `profiles`. The picker row + the positional `hyprpilot <id>`.                                                   |
| `agent`         | string                           | —       | Which `agents` entry to launch. Must reference a real `agents[].id`.                                                          |
| `model`         | string (optional)                | unset   | Overrides the agent's default model. Precedence: profile > agent > vendor default.                                            |
| `effort`        | string (optional)                | unset   | Reasoning-effort knob, mapped to the vendor's config surface where supported.                                                 |
| `cwd`           | path (optional)                  | unset   | Where the agent runs. `~`, `${VAR}` expansion supported; falls back to the agent `cwd`, then `$PWD`.                          |
| `mode`          | string (optional)                | unset   | Vendor-specific starting mode. See [Agents → Modes](./agents#modes).                                                          |
| `headless`      | bool (optional)                  | `false` | Force a non-interactive one-shot launch (requires a piped prompt). See [Headless](#headless).                                 |
| `system_prompt` | `{ file, inject? }[]` (optional) | unset   | Prompt files read at resolve time and prepended to the first turn. `[]` = no prompt. `inject` defaults `true`.                |
| `mcps`          | `{ file … }[]` (optional)        | unset   | Per-profile MCP catalogue — wholesale-replaces the shared set. `[]` = no MCPs. See [MCP](./mcp).                              |
| `mcp`           | `mcp` block (optional)           | unset   | Per-profile override of the in-tree MCP / skills block — wholesale-replaces the global.                                       |
| `command`       | string (optional)                | unset   | Replaces the base agent's `command` wholesale for this profile.                                                               |
| `args`          | string[] (optional)              | unset   | Replaces the base agent's `args` wholesale for this profile.                                                                  |
| `env`           | map (optional)                   | `{}`    | Overlays the base agent's `env` per key; the profile's key wins on collision.                                                 |
| `harness`       | `{ enabled? }` (optional)        | unset   | Per-profile [agent-harness](../runtime/harness) policy. **Opt-in** — without this block the harness cannot drive the profile. |

## What a profile overrides

At resolve time the profile's `model` / `effort` / `mode` / `cwd` override the agent entry — the profile is the more specific scope. Model precedence, for example, is **profile > agent > vendor default**. The `--mode` / `--cwd` flags then override the resolved profile per launch; for a one-off `model` (or any other profile field) use [`--with-config`](../runtime/with-config) — there is no `--model` flag.

## The flat `command` / `args` / `env` override

Sometimes a profile needs to launch a _different_ binary or extra flags than the base agent entry declares — a canary build, a wrapper script, a long flag list. Instead of a nested override block, a profile carries three flat top-level fields:

```yaml
profiles:
  - id: engineer-canary
    agent: claude-code
    command: claude-canary # REPLACES the agent's command
    args: # REPLACES the agent's args
      - --dangerously-skip-permissions
    env:
      ANTHROPIC_LOG: debug # OVERLAYS the agent's env per-key
```

- **`command`** — when set, replaces the base agent's `command` wholesale for this profile.
- **`args`** — when set, replaces the base agent's `args` wholesale. Flags have no stable key to merge by (`--flag value`, `-c k=v`, positionals), so this is a swap, not an append: to add one flag to an otherwise long agent-args list, restate the full list here.
- **`env`** — overlays onto the base agent's `env` per key. The profile's key wins on collision; keys the profile doesn't mention are left untouched.

The agent's `provider` still drives the native-flag projection (model, mode, MCP config); the flat override only swaps what binary is launched with what arguments and environment.

## System prompts

`system_prompt` is an array of `{ file, inject? }` entries. Each file is read at **resolve** time (not ahead of time), the surviving bodies are concatenated with blank-line separators, and the result is prepended to your first turn so the agent reads it as context before your message.

```yaml
system_prompt:
  - file: ~/.config/hyprpilot/prompts/base.md # shared persona
  - file: ~/.config/hyprpilot/prompts/engineer.md # per-profile addendum
```

Composition lets a base persona + per-profile addendum land without juggling templates. `system_prompt: []` is the explicit "no prompt" off-switch. Because files are read at resolve time, a missing file fails loudly on the next launch rather than silently.

### Per-entry inject toggle

Each entry takes an optional `inject` boolean (default `true`). Set it `false` to keep a file listed — for reference, or to stage it for later — without its body actually being injected:

```yaml
system_prompt:
  - file: ~/.config/hyprpilot/prompts/base.md
  - file: ~/.config/hyprpilot/prompts/notes.md
    inject: false # skipped
```

| Field    | Type            | Default | What it does                                                          |
| -------- | --------------- | ------- | --------------------------------------------------------------------- |
| `file`   | path            | —       | Prompt file, read at resolve time — a missing file fails the launch.  |
| `inject` | bool (optional) | `true`  | Whether this entry's body rides the launch's system-prompt injection. |

## Putting a profile on the harness

`mcp harness` lets a connected agent launch your profiles. **It is opt-in per profile** — declaring a `harness` block is what makes one available:

```yaml
profiles:
  - id: personal/engineer
    agent: claude-code
    harness:
      enabled: true # an agent may drive this one

  - id: personal/deploy
    agent: claude-code # no block — the harness cannot touch it
```

A profile without the block, or with `enabled: false`, disappears from `list_profiles` **and** is refused by `spawn` / `session_send` by id. Both halves matter: `spawn` dispatches on whatever id it is handed, so hiding a profile from the listing alone would leave it reachable by anyone who already knew the name.

Default-deny because `spawn` runs a profile's `command` as you — the set an agent may drive should be a list you wrote, not everything that happens to be configured.

To opt a whole family in at once, use a `$match`ed [patch](./patches) instead of repeating the block:

```yaml
patches:
  - $match:
      profile: 'personal/*'
    harness:
      enabled: true
```

This is a _harness_ policy, not a hidden flag — `hyprpilot profiles` still lists every profile and `hyprpilot personal/deploy` still launches it. It says "an agent may not drive this one", not "nobody may".

It is a block rather than a bare `harness: true` so later per-profile harness policy lands as a sibling field instead of a second top-level flag.

## Headless

`headless: true` forces the profile to launch **non-interactively** — hyprpilot buffers stdin as the prompt and projects the vendor's one-shot invocation (`claude --print` / `codex exec` / `opencode run`), then the vendor exits:

```yaml
profiles:
  - id: commit-msg
    agent: claude-code
    headless: true
    system_prompt:
      - file: ~/.config/hyprpilot/prompts/commit.md
```

```sh
git diff --staged | hyprpilot commit-msg
```

The flag defaults `false` (interactive TUI). Note that a piped stdin **auto-triggers** headless regardless of this flag — `headless: true` is only needed when you want a profile to _refuse_ an interactive launch. When headless is active but stdin is an interactive TTY (no prompt to read), the launch errors instead of opening a picker it can't drive. Full details, per-vendor projection, and the `-- …` escape hatch live in [Runtime → Launching → Headless](../runtime/launch#headless-stdin-pass-through).

## MCPs and skills

MCP servers extend an agent with tools; skills attach markdown context. See [MCP](./mcp) and [Runtime → Skills](../runtime/skills). In short:

- `mcps` on a profile is a per-profile MCP catalogue that wholesale-replaces the shared set; `mcps: []` means "no MCPs".
- The in-tree `hyprpilot` MCP server (which delivers your skills) is configured under the `mcp` block, seeded globally via [`patches`](./patches) and overridable per-profile.

## Examples

### A planning profile with no MCPs

```yaml
profiles:
  - id: plan
    agent: claude-code
    model: claude-opus-4-5
    mode: plan
    mcps: []
```

### A code-review profile pinned to a repo

```yaml
profiles:
  - id: review-hyprpilot
    agent: claude-code
    cwd: ~/code/hyprpilot
    system_prompt:
      - file: ~/.config/hyprpilot/prompts/reviewer.md
```

### A profile launching a wrapper binary

```yaml
profiles:
  - id: sandboxed
    agent: claude-code
    command: firejail
    args:
      - --net=none
      - claude
```
