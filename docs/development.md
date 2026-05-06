---
title: Development
order: 99
---

# Development

Working on hyprpilot itself. The build orchestrator is `task` ([go-task](https://taskfile.dev)); the toolchain is pinned via `mise`.

## Toolchain

`mise install` at the repo root drops the required versions:

| Tool | Version | Why |
| --- | --- | --- |
| `rust` | stable + `rustfmt` + `clippy` | Daemon binary. |
| `node` | 24 | UI build. |
| `pnpm` | 10 | Workspace manager. |
| `task` | 3 | Build orchestrator. |
| `cargo-nextest` | latest | Rust test runner (`task test` drives this). |

`rust-toolchain.toml` covers `cargo` invocations outside mise.

## Tasks

| Task | What it does |
| --- | --- |
| `task install` | `cargo fetch` + `pnpm install` across every workspace. |
| `task dev` | Tauri dev cycle: Vite + Tauri + hot-reload. |
| `task test` | UI Vitest + Rust nextest. E2E stays out of the inner loop. |
| `task test:ui` | Vitest only. |
| `task test:e2e` | Playwright e2e (browser mode). `HYPRPILOT_E2E_MODE=tauri` for the bridge path (gated). |
| `task format` | Rust `cargo fmt` + UI Prettier + ESLint `--fix`. |
| `task lint` | `cargo fmt --check` + `cargo clippy -D warnings` + ESLint + `vue-tsc --noEmit`. |
| `task build` | Debug build via `tauri build --debug`. **Gold-standard pre-push verification** — if this passes, CI passes. |
| `task release` | Release build via `tauri build`. |
| `task docs:dev` | VitePress dev server (this site). |
| `task docs:build` | Production docs build. |

## Workspace layout

```
hyprpilot/
├── src-tauri/                Rust crate — daemon + ctl + ACP bridge
│   ├── src/
│   │   ├── adapters/         ACP transport + agent registry
│   │   ├── config/           layered TOML loader (defaults + global + profile + CLI)
│   │   ├── daemon/           Tauri lifecycle, layer-shell, tray
│   │   ├── rpc/              JSON-RPC dispatch on the unix socket
│   │   ├── ctl/              CLI subcommand handlers
│   │   ├── tools/            sandbox / fs / terminal primitives
│   │   └── formatting/       tool-call formatter registry
│   └── icons/                Tauri bundle icons (LFS-tracked)
├── ui/                       Vue 3 + Vite + Tailwind v4
│   ├── src/
│   │   ├── components/       reusable building blocks
│   │   ├── views/            feature partials (chat, palette, composer, …)
│   │   ├── composables/      cross-feature state (theme, keymaps, instances)
│   │   ├── interfaces/       wire types
│   │   ├── ipc/              Tauri command + event bridge
│   │   └── lib/              pure helpers (no Vue, no Tauri)
│   └── tests/                test scaffolding (dev-preview shim, fixtures)
├── tests/e2e/                Playwright tests
├── docs/                     this VitePress site
├── packaging/                AUR + desktop + systemd assets
└── .github/workflows/        CI / release-please / release / package / docs
```

## Testing tiers

Two runners, two locations:

| Tier | Runner | Lives | Suffix |
| --- | --- | --- | --- |
| Component / composable / lib | Vitest + jsdom | beside the source | `*.test.ts` |
| End-to-end | Playwright via `@srsholmes/tauri-playwright` | `tests/e2e/specs/` | `*.spec.ts` |

Component tests mock Tauri IPC by replacing the `@ipc` barrel with `vi.mock('@ipc', ...)`. Don't monkey-patch `window.__TAURI__`.

E2E specs run in **browser mode** by default (Vite + Chromium + IPC mocks). The native-Tauri path (`HYPRPILOT_E2E_MODE=tauri`) is fully wired but stalls on webkit2gtk-4.1 — opens up when GTK4 lands upstream.

## Verifying changes

`task build` is the gold standard. It runs:

1. UI: `vue-tsc --noEmit` + `vite build`.
2. Rust: `cargo build --debug`.

If `task build` exits 0, anything that lands on `main` will pass CI. Don't claim "vue-tsc clean" on the back of `pnpm exec vue-tsc` — `pnpm exec` silently exits 0 when the binary isn't in the workspace root's `.bin/`. **Always use named pnpm scripts or task targets.**

## Logging

`tracing` is bootstrapped via `logging::init`. Filter precedence: `--log-level` → `RUST_LOG` → `info` fallback.

Targeted trace channels (silent by default):

| Target | What it captures |
| --- | --- |
| `acp::wire` | Every incoming `session/update` notification + every outgoing `session/prompt` request (raw JSON). |
| `acp::thought` | Per-`agent_thought_chunk` extraction outcome. |

```sh
RUST_LOG='hyprpilot::adapters=info,acp::wire=trace' hyprpilot daemon
```

Logs land at `$XDG_STATE_HOME/hyprpilot/logs/hyprpilot.log.<date>`.

## Conventions

- **Conventional commits** with optional `refs K-<id>` / `closes K-<id>` trailers for Linear linking.
- **Branch prefixes:** `feat/`, `fix/`, `chore/`, `docs/`, `ci/`, `refactor/`.
- **Squash-merge only** on `main`. Branches auto-delete after merge.
- **No direct pushes** to `main` — branch protection is on. Open a PR; review optional for solo dev.

For deeper architectural rules (no backwards-compat layers, components compose / don't bag, traits-vs-enums, naming) see `CLAUDE.md` at the repo root — that's the agent operating manual and the source of truth for codebase rules.
