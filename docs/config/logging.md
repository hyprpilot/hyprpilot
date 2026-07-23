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
| `level` | enum | _unset_ | One of `trace` / `debug` / `info` / `warn` / `error`. Applied only when `--log-level` and `RUST_LOG` are both unset. |

`logging.level` is **not seeded** — leaving it unset lets the built-in `error` filter (below) own the default, so a fresh run is quiet (errors only) until you ask for more. Seeding a level in the compiled defaults would re-nullify the scoped `logging.level` filter, so the code fallback owns the default; set `logging.level` in your own config to raise verbosity.

The filter is resolved from the loaded config **before** the tracing subscriber is installed, so `logging.level` (and the other sources) take effect on the very first line — including the "config loaded" line. Set `level: error` (or run with `--log-level error`) and hyprpilot emits nothing below `error`.

## Filter precedence

The active filter is resolved from four sources, highest first:

1. `--log-level <level>` (or `HYPRPILOT_LOG_LEVEL`) — a single level: `trace`, `debug`, `info`, `warn`, or `error`.
2. `RUST_LOG` — a full [`tracing` env-filter expression](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html), for per-crate targeting.
3. `logging.level` in config — folded into the filter once the config is loaded, only when neither of the above spoke.
4. The built-in default — `error`, which keeps a fresh run quiet: only errors surface unless a level is explicitly requested.

If you want file/line provenance on each log line, run with `--log-level debug` (or `trace`) — the `file:line` tagging rides only on those levels so the narrative stays terse.

## Why logs stop at exec

Hyprpilot `exec()`s into the vendor CLI, so its own tracing only covers the brief resolve phase before hand-off: config load, profile resolution, MCP/skills registry construction, and the projection. Once the vendor TUI takes over the terminal, everything you see belongs to the vendor.

That makes `--log-level debug` the go-to for launch problems — you get the whole resolve narrative, then the vendor starts (or the error that stopped it) with nothing else in between:

```sh
hyprpilot engineer --log-level debug
```
