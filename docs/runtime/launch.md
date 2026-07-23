---
title: Launching
order: 30
---

# {{ $frontmatter.title }}

The bare invocation **is** the launch — hyprpilot is one binary, there is no `run` subcommand, and two subcommands (`profiles`, `mcp serve`) round out the surface:

```sh
hyprpilot [--profile <id>] [flags] [-- <provider args>]
hyprpilot profiles [--json]
hyprpilot mcp serve [--skill-dir <json>]…
```

<!-- more -->

## Picking a profile

```sh
hyprpilot                       # interactive picker over configured profiles
hyprpilot -p engineer           # launch the `engineer` profile directly
```

If you omit `--profile`/`-p` and no `profile.default` is set, an interactive fuzzy picker (powered by `nucleo`) opens over your configured profiles — each row shows the default marker, id, agent, model, and cwd. Cancelling the picker aborts the launch; a non-interactive terminal errors instead of hanging.

## Launch flags

| Flag                                      | Purpose                                                                                              |
| ----------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| `-p`, `--profile <id>`                    | Session profile to resolve and launch.                                                               |
| `--agent <id>`                            | Swap the profile's agent entry for this launch — wins over the (patched) profile's agent.            |
| `--cwd <dir>`                             | Working directory for the vendor process.                                                            |
| `--mode <mode>`                           | Mode override, projected onto the vendor where supported.                                            |
| `--model <model>`                         | Model override, projected onto the vendor where supported.                                           |
| `--with-config <path\|@inline\|->`        | Profile overlay patch (repeatable). See [Ad-hoc Overlays](./with-config).                            |
| `--with-config-format <toml\|json\|yaml>` | Format for stdin / inline / extension-less overlays (default `json`).                                |
| `-- <args>`                               | Everything after `--` is forwarded verbatim to the vendor CLI; generated equivalents are suppressed. |

## Overriding per launch

If you want to deviate from the profile for one launch, the launch flags override the resolved profile without touching your config:

```sh
hyprpilot -p engineer --agent codex           # swap the agent entry wholesale
hyprpilot -p engineer --cwd ~/code/foo        # run somewhere else
hyprpilot -p engineer --model claude-opus-4-5 # different model
hyprpilot -p engineer --mode plan             # vendor-specific mode
```

- `--agent <id>` swaps which [`agents`](../config/agents) entry the launch uses — it wins over whatever agent the (patched) profile names.
- `--cwd <dir>` beats the profile's (or agent's) configured `cwd`; when neither speaks, the current directory is used.
- `--model` / `--mode` are projected onto the vendor CLI where supported.

For structural one-off overrides — a different MCP set, an extra system prompt — reach for [`--with-config`](./with-config) instead.

## Forwarding native arguments

Everything after a `--` separator is forwarded verbatim to the vendor CLI — use it for provider-native flags and resume flows:

```sh
hyprpilot -p engineer -- --resume
hyprpilot -p review -- --dangerously-skip-permissions
```

Any provider-native argument you pass this way suppresses hyprpilot's generated equivalent, so your hand-written flag always wins over the projection.

## Global flags

Available on every invocation, each with an environment twin, so you can pin them per shell or per session:

| Flag                      | Env                        | Purpose                                                                      |
| ------------------------- | -------------------------- | ---------------------------------------------------------------------------- |
| `--config <path>`         | `HYPRPILOT_CONFIG`         | Override the global config path (format inferred from the extension).        |
| `--config-profile <name>` | `HYPRPILOT_CONFIG_PROFILE` | Layer a named config-layer overlay (`profiles/<name>.{ext}`).                |
| `--log-level <level>`     | `HYPRPILOT_LOG_LEVEL`      | Override the tracing filter (`trace` / `debug` / `info` / `warn` / `error`). |

```sh
HYPRPILOT_CONFIG_PROFILE=work hyprpilot -p engineer
```

`--config-profile` layers a named config file (`~/.config/hyprpilot/profiles/<name>.yaml`) on top of your global config — a **config-layer** profile, distinct from the session `profiles` you address with `-p`. See [Config → Layering](../config/layering).

Log filter precedence is `--log-level` → `RUST_LOG` → `logging.level` → the built-in `warn,hyprpilot=info` default; tracing always writes to stderr. See [Config → Logging](../config/logging).

## What a launch does

1. Load + validate layered config ([Config → Layering](../config/layering)).
2. Pick the profile (`-p` → `profile.default` → picker) and fold [`patches`](../config/patches) + [`--with-config`](./with-config) overlays.
3. Build the per-launch MCP + skills registries, auto-injecting the `hyprpilot` server when skills resolve ([Skills](./skills)).
4. Project everything onto the vendor's native flags/env ([Config → Agents](../config/agents)).
5. Optionally rename the tmux window / zellij tab ([Config → Multiplexer](../config/multiplexer)).
6. `exec()` — the vendor CLI replaces the hyprpilot process.

### cwd precedence

The working directory the vendor launches in resolves as: explicit `--cwd` flag → the profile's (or agent's) configured `cwd` → the current directory. A profile pinned to a repo therefore launches there by default, and `--cwd` overrides it per invocation.

## Inspecting without launching

```sh
hyprpilot profiles              # table: default marker, profile, agent, model
hyprpilot profiles --json       # machine-readable
```

The listing resolves config the same way a launch does — including [`patches`](../config/patches) — but stops before exec, so the displayed summaries reflect what a launch _would_ use. `--json` keeps stdout pure (all tracing goes to stderr), safe to pipe into `jq`.

An empty `profiles` list is a validation error, not an empty table — fresh installs refuse to run until you configure at least one profile ([Quickstart](./quickstart)). A config typo aborts with an error naming the offending field path.

## Exit behavior

Because a successful launch replaces the process, hyprpilot's own exit code is the vendor CLI's on unix (non-unix platforms fall back to spawn-and-propagate-exit-code). Config load failures, an empty `profiles` list, an unresolvable profile, or a missing `system_prompt` file abort before exec with a readable error naming the problem.
