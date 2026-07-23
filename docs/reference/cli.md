---
title: CLI
order: 1
---

# CLI reference

Hyprpilot is one binary. The bare invocation *is* the launch; two
subcommands round out the surface.

```sh
hyprpilot [--profile <id>] [flags] [-- <provider args>]
hyprpilot profiles [--json]
hyprpilot mcp serve [--skill-dir <json>]…
```

## Bare launch

Running `hyprpilot` with no subcommand resolves a profile and `exec()`s
into the vendor CLI, replacing hyprpilot's own process. On unix the exec
is a true process replacement (no child); elsewhere it falls back to
spawn-and-propagate-exit-code.

```sh
hyprpilot                       # pick a profile interactively, then exec
hyprpilot -p engineer           # launch the `engineer` profile
hyprpilot --profile engineer --cwd ~/code/foo
hyprpilot -p engineer --model claude-opus-4-5
hyprpilot -p engineer -- --resume   # everything after -- is forwarded verbatim
```

When `--profile`/`-p` is omitted and no default resolves, an interactive
`nucleo` picker over your configured profiles opens.

### Launch flags

| Flag | Purpose |
| --- | --- |
| `-p`, `--profile <id>` | Session profile to resolve and launch. |
| `--agent <id>` | Swap the profile's agent entry for this launch. |
| `--cwd <dir>` | Working directory for the vendor process. |
| `--mode <mode>` | Mode override, projected onto the vendor where supported. |
| `--model <model>` | Model override, projected onto the vendor where supported. |
| `--with-config <path\|@inline\|->` | Profile overlay patch (repeatable). |
| `--with-config-format <toml\|json\|yaml>` | Format for stdin / inline / extension-less overlays (default `json`). |
| `-- <args>` | Everything after `--` is forwarded verbatim to the vendor CLI. |

### cwd precedence

The working directory the vendor launches in is resolved as: explicit
`--cwd` flag → the profile's (or agent's) configured `cwd` → the current
directory. A profile pinned to a repo therefore launches there by
default, and `--cwd` overrides it per invocation.

## Global flags

Available on every invocation:

| Flag | Env | Purpose |
| --- | --- | --- |
| `--config <path>` | `HYPRPILOT_CONFIG` | Override the global config path (format inferred from the extension). |
| `--config-profile <name>` | `HYPRPILOT_CONFIG_PROFILE` | Layer a named config-layer overlay (`profiles/<name>.{ext}`). |
| `--log-level <level>` | `HYPRPILOT_LOG_LEVEL` | Override the tracing filter (`trace`…`error`). |

Log filter precedence is `--log-level` → `RUST_LOG` → `[logging] level`
→ the built-in default. Tracing always writes to stderr; because
hyprpilot `exec()`s into the vendor, its own logging only covers the
brief resolve phase before hand-off.

## `hyprpilot profiles`

Lists configured session profiles, applying root `[[patches]]` to the
displayed summaries. Reads local config only.

```sh
hyprpilot profiles              # table: default marker, id, agent, model
hyprpilot profiles --json       # machine-readable
```

## `hyprpilot mcp serve`

Runs the in-tree MCP server over stdio. **You don't run this by hand** —
the agent vendor spawns it as a child when hyprpilot auto-injects the
`hyprpilot` MCP entry (see [MCP & skills](../configuration/mcp-and-skills)).
It is documented for completeness on the
[MCP server reference](./mcp-server) page.

```sh
hyprpilot mcp serve --skill-dir '{"dir":"/abs/path","ignore":[]}'
```

## Exit behavior

Because a successful launch replaces the process, hyprpilot's own exit
code is the vendor CLI's on unix. Config load failures, an empty
`[[profiles]]` list, an unresolvable profile, or a missing
`system_prompt` file abort before exec with a readable error naming the
problem.
