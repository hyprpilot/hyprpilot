---
title: Development
order: 3
---

# Development

Curious how hyprpilot is put together, or want to try a change locally? Here's the short version.

## Getting it running

Hyprpilot is a Rust binary with a Vue frontend, glued together by [Tauri](https://tauri.app). The toolchain is pinned through [`mise`](https://mise.jdx.dev), and [`task`](https://taskfile.dev) drives everything you'll typically run.

```sh
git clone https://github.com/hyprpilot/hyprpilot
cd hyprpilot
mise install
task install
task dev
```

`task dev` opens the overlay with hot reload — edit a Vue component or a Rust file and the change shows up without a manual rebuild. `task test` runs the tests, `task lint` runs the linters, and `task --list` shows the rest.

## Where things live

The Rust crate lives under `src-tauri/` and produces a single binary that doubles as the daemon (`hyprpilot daemon`) and the CLI client (`hyprpilot ctl`); the two halves talk over a unix socket. The Vue UI lives under `ui/`. Everything else is fairly self-explanatory once you poke around.

## Found a rough edge?

Open an issue, send a PR, or just drop a thought in [Discussions](https://github.com/hyprpilot/hyprpilot/discussions). Small, focused changes are the easiest to review and land — but don't worry about getting it perfect; we can always iterate.
