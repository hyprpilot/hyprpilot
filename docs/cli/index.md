---
title: CLI Overview
order: 1
prev: false
---

# {{ $frontmatter.title }}

Hyprpilot is one binary. The bare invocation _is_ the launch; two subcommands round out the surface.

<!-- more -->

```sh
hyprpilot [--profile <id>] [flags] [-- <provider args>]
hyprpilot profiles [--json]
hyprpilot mcp serve [--skill-dir <json>]…
```

| Invocation                           | Purpose                                                            |
| ------------------------------------ | ------------------------------------------------------------------ |
| [`hyprpilot`](./launch)              | Resolve a profile and `exec()` into the vendor CLI.                |
| [`hyprpilot profiles`](./profiles)   | List configured session profiles without launching.                |
| [`hyprpilot mcp serve`](./mcp-serve) | The in-tree MCP server — spawned by the agent vendor, not by hand. |

## Global flags

Available on every invocation, each with an environment twin:

| Flag                      | Env                        | Purpose                                                                      |
| ------------------------- | -------------------------- | ---------------------------------------------------------------------------- |
| `--config <path>`         | `HYPRPILOT_CONFIG`         | Override the global config path (format inferred from the extension).        |
| `--config-profile <name>` | `HYPRPILOT_CONFIG_PROFILE` | Layer a named config-layer overlay (`profiles/<name>.{ext}`).                |
| `--log-level <level>`     | `HYPRPILOT_LOG_LEVEL`      | Override the tracing filter (`trace` / `debug` / `info` / `warn` / `error`). |

Log filter precedence is `--log-level` → `RUST_LOG` → `[logging] level` → the built-in `warn,hyprpilot=info` default. Tracing always writes to stderr; because hyprpilot `exec()`s into the vendor, its own logging only covers the brief resolve phase before hand-off. See [Features → Logging](../features/logging).

## Exit behavior

Because a successful launch replaces the process, hyprpilot's own exit code is the vendor CLI's on unix (non-unix platforms fall back to spawn-and-propagate-exit-code). Config load failures, an empty `[[profiles]]` list, an unresolvable profile, or a missing `system_prompt` file abort before exec with a readable error naming the problem.
