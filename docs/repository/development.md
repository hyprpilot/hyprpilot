---
title: Development
order: 3
---

# Development

The build orchestrator is `task` ([go-task](https://taskfile.dev)); the toolchain is pinned via `mise`.

## Getting set up

```sh
git clone https://github.com/hyprpilot/hyprpilot
cd hyprpilot
mise install   # rust, node, pnpm, task
task install   # cargo fetch + pnpm install across the workspace
```

## Tasks

| Task | What it does |
| --- | --- |
| `task dev` | Tauri dev cycle — Vite + Tauri + hot reload. |
| `task build` | Debug build. The pre-push gold standard — if this passes, CI passes. |
| `task test` | UI Vitest + Rust nextest. |
| `task lint` | Format + clippy + ESLint + `vue-tsc`. |
| `task format` | Auto-fix everything `task lint` checks. |
| `task docs:dev` | Run this site locally. |

`task --list` shows the rest.

## Layout

```
src-tauri/   Rust crate — daemon, ctl, ACP bridge, config loader, formatter registry
ui/          Vue 3 + Vite + Tailwind v4 frontend
tests/e2e/   Playwright tests
docs/        this VitePress site
packaging/   AUR + desktop + systemd assets
```

The Rust side is one binary that doubles as the daemon (`hyprpilot daemon`) and the CLI client (`hyprpilot ctl`). They talk over a unix socket at `$XDG_RUNTIME_DIR/hyprpilot.sock`.

## Logs

`tracing` writes a rolling file to `$XDG_STATE_HOME/hyprpilot/logs/hyprpilot.log.<date>`. Override the level via `RUST_LOG`:

```sh
RUST_LOG='hyprpilot=debug' hyprpilot daemon
```

When running under systemd: `journalctl --user -u hyprpilot.service -f`.

## Conventions

The agent operating manual at the repo root (`CLAUDE.md`) is the source of truth for naming, error handling, type design, and component shape. Keep it open while working — it's where every "should this be a trait or an enum?" decision lands.
