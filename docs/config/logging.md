---
title: Logging
order: 70
next: false
---

# {{ $frontmatter.title }}

Hyprpilot logs through `tracing`, **always to stderr** — debug and release builds alike, ANSI colors on. Stdout stays clean for machine-readable output like `profiles --json`.

<!-- more -->

## Configuration

```yaml
logging:
  level: info # trace | debug | info | warn | error
```

| Field   | Type | Default | What it does                                                                                                         |
| ------- | ---- | ------- | -------------------------------------------------------------------------------------------------------------------- |
| `level` | enum | `info`  | One of `trace` / `debug` / `info` / `warn` / `error`. Applied only when `--log-level` and `RUST_LOG` are both unset. |

## Filter precedence

The active filter is resolved from four sources, highest first:

1. `--log-level <level>` (or `HYPRPILOT_LOG_LEVEL`) — a single level: `trace`, `debug`, `info`, `warn`, or `error`.
2. `RUST_LOG` — a full [`tracing` env-filter expression](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html), for per-crate targeting.
3. `logging.level` in config — applied once the config is loaded, only when neither of the above spoke.
4. The built-in default — `warn,hyprpilot=info`, which keeps third-party crates (tokio, rmcp, nucleo) at `warn` while surfacing hyprpilot's own `info` lifecycle narrative.

If you want file/line provenance on each log line, run with `--log-level debug` (or `trace`) — the `file:line` tagging rides only on those levels so the `info` narrative stays terse.

## Why logs stop at exec

Hyprpilot `exec()`s into the vendor CLI, so its own tracing only covers the brief resolve phase before hand-off: config load, profile resolution, MCP/skills registry construction, and the projection. Once the vendor TUI takes over the terminal, everything you see belongs to the vendor.

That makes `--log-level debug` the go-to for launch problems — you get the whole resolve narrative, then the vendor starts (or the error that stopped it) with nothing else in between:

```sh
hyprpilot -p engineer --log-level debug
```
