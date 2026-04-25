# hyprpilot

Rust Tauri 2 overlay daemon + unix-socket CLI for agent-driven workflows on
Hyprland / Sway. Port of the Python `pilot.py` stack living under
[`cenk/dotfiles`](https://gitlab.kilic.dev/cenk/dotfiles/-/tree/master/wayland/.config/wayland/scripts).

## Quick start

```sh
mise install         # pin rust / node / pnpm / task
task install         # cargo fetch + pnpm install
task dev             # launch the Tauri app with hot-reload

./target/debug/hyprpilot            # run the daemon (default subcommand)
./target/debug/hyprpilot ctl --help # invoke the CLI surface
```

See `CLAUDE.md` for the full agent-facing manual (toolchain, tasks,
config layering, logging, framework quirks).

Compositor / panel integration:

- `docs/hyprland.md` — recommended `bind = SUPER, space, exec, hyprpilot
  ctl overlay toggle` keybind + the full `overlay/*` surface.
- `docs/waybar.md` — `custom/hyprpilot` waybar module driven by
  `ctl status --watch`.

## Layout

```
hyprpilot/
├── Cargo.toml            # Rust workspace manifest
├── package.json          # pnpm workspace root (devDep: @tauri-apps/cli)
├── pnpm-workspace.yaml   # workspace packages: ui, tests/e2e, tests/e2e/support/mock-agent
├── src-tauri/            # Rust crate (clap + Tauri 2 + tokio unix socket)
├── ui/                   # Vue 3 + Vite + Tailwind + shadcn-vue frontend
├── tests/e2e/            # Playwright e2e suite + scripted mock-agent
├── mise.toml             # toolchain pins
├── Taskfile.yml          # install / dev / test / format / lint / build / release
└── .mcp.json             # repo-scoped MCP server registry (empty by default)
```
