---
title: Development
order: 10
---

# {{ $frontmatter.title }}

Hyprpilot is a single Rust crate at the repo root — no frontend, no webview, no node runtime beyond the docs site. The toolchain is pinned through [`mise`](https://mise.jdx.dev), and [`task`](https://taskfile.dev) drives everything you'll typically run.

<!-- more -->

## Getting it running

```sh
git clone https://github.com/hyprpilot/hyprpilot
cd hyprpilot
mise install
task build
```

`mise install` drops the pinned toolchain: Rust (stable + `rustfmt` + `clippy`), `task`, `cargo-nextest`, plus node + pnpm for the `docs/` VitePress site — the only Node consumer in the repo.

## Tasks

| Task                                            | Purpose                                                               |
| ----------------------------------------------- | --------------------------------------------------------------------- |
| `task install`                                  | `cargo fetch` + `pnpm install`.                                       |
| `task build`                                    | Debug build of the launcher.                                          |
| `task release`                                  | Release build.                                                        |
| `task test`                                     | Rust test suite via `cargo nextest`.                                  |
| `task lint`                                     | `cargo fmt --check` + `cargo clippy -D warnings`, plus the docs lint. |
| `task format`                                   | `cargo fmt --all`, plus the docs formatter.                           |
| `task run -- <args>`                            | `cargo run` with launcher args.                                       |
| `task docs:dev` / `docs:build` / `docs:preview` | VitePress docs site.                                                  |

The pre-push bar is `task build && task lint && task test` — all green. CI runs lint, test, and build as separate jobs.

## Where things live

The crate is a single package at the repo root (`Cargo.toml` + `src/`). Key modules:

- `src/main.rs` — the `clap`-derive CLI. Bare invocation launches; `mcp` / `profiles` are the only subcommands.
- `src/config/` — layered config load, merge, validation, `[[agents]]` / `[[profiles]]`, patches, and the compiled `defaults.toml`.
- `src/resolve/` — the pure `Config` → resolution core (profile pick, patch folding, per-launch MCP + skills registries).
- `src/spawn/` — profile launch: per-vendor native-flag projection, the interactive picker, the multiplexer rename, and the final `exec()`.
- `src/mcp/` — the MCP catalogue plus the in-tree `hyprpilot mcp serve` server.
- `src/skills/` — the `SKILL.md` loader and registry.

## Found a rough edge?

Open an issue, send a PR, or drop a thought in [Discussions](https://github.com/hyprpilot/hyprpilot/discussions). Small, focused changes are the easiest to review and land — see [Contributions](./contributions).
