---
title: hyprpilot (launch)
order: 10
---

# {{ $frontmatter.title }}

Running `hyprpilot` with no subcommand resolves a profile and `exec()`s into the vendor CLI, replacing hyprpilot's own process.

<!-- more -->

```sh
hyprpilot                       # pick a profile interactively, then exec
hyprpilot -p engineer           # launch the `engineer` profile
hyprpilot --profile engineer --cwd ~/code/foo
hyprpilot -p engineer --model claude-opus-4-5
hyprpilot -p engineer -- --resume   # everything after -- is forwarded verbatim
```

When `--profile`/`-p` is omitted and no `[profile] default` resolves, an interactive `nucleo` fuzzy picker over your configured profiles opens. A non-interactive terminal errors instead of hanging; cancelling the picker aborts the launch.

## Launch flags

| Flag                                      | Purpose                                                                                              |
| ----------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| `-p`, `--profile <id>`                    | Session profile to resolve and launch.                                                               |
| `--agent <id>`                            | Swap the profile's agent entry for this launch — wins over the (patched) profile's agent.            |
| `--cwd <dir>`                             | Working directory for the vendor process.                                                            |
| `--mode <mode>`                           | Mode override, projected onto the vendor where supported.                                            |
| `--model <model>`                         | Model override, projected onto the vendor where supported.                                           |
| `--with-config <path\|@inline\|->`        | Profile overlay patch (repeatable). See [Ad-hoc Overlays](../features/with-config).                  |
| `--with-config-format <toml\|json\|yaml>` | Format for stdin / inline / extension-less overlays (default `json`).                                |
| `-- <args>`                               | Everything after `--` is forwarded verbatim to the vendor CLI; generated equivalents are suppressed. |

## cwd precedence

The working directory the vendor launches in resolves as: explicit `--cwd` flag → the profile's (or agent's) configured `cwd` → the current directory. A profile pinned to a repo therefore launches there by default, and `--cwd` overrides it per invocation.

## What a launch does

1. Load + validate layered config ([Config Layering](../features/layering)).
2. Pick the profile (`-p` → `[profile] default` → picker) and fold `[[patches]]` + `--with-config` overlays.
3. Build the per-launch MCP + skills registries, auto-injecting the `hyprpilot` server when skills resolve ([Skills](../features/skills)).
4. Project everything onto the vendor's native flags/env ([Agents](../features/agents)).
5. Optionally rename the tmux window / zellij tab ([Multiplexer Title](../features/multiplexer)).
6. `exec()` — the vendor CLI replaces the hyprpilot process.
