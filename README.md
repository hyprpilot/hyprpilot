# hyprpilot

**Launch your terminal coding agent, pre-configured — resolve a session profile from layered config, project it onto the vendor's native CLI, and get out of the way.**

---

## Documentation

**[Read the documentation...](https://hyprpilot.kilic.dev)**

## What it does

hyprpilot is a config-driven, fire-and-exec launcher for terminal coding agents — a single Rust binary. No daemon, no socket, no UI.

- Resolves a **profile** from layered TOML / JSON / YAML config (compiled defaults → global config → named config-layer), folding root `[[patches]]` and ad-hoc `--with-config` overlays.
- Projects the profile's model / effort / mode / system-prompt / MCP catalogue / tool-policy onto the chosen vendor's **native** CLI — `claude`, `codex`, or `opencode`.
- `exec()`s into that vendor CLI, replacing its own process, after optionally renaming the tmux window / zellij tab.

Pick a profile from an interactive fuzzy picker, name it directly (`hyprpilot engineer`), or run headless — `echo "fix the test" | hyprpilot engineer`, `hyprpilot engineer --prompt "…"`, or `hyprpilot engineer --file task.md`. Skills reach the agent over an in-tree MCP server the launcher auto-injects.

## DISCLAIMER

This is a ~vibe-coded~ agentically engineered, sorry, ~pos~, sorry, poc project that I have used to sharpen my skills to build a thing from start to finish. It started life as a basic GTK overlay, grew a background daemon, and has since been stripped all the way back down to what you see here: a boring little launcher that reads some config and `exec`s into your agent. Turns out that is the part I actually keep using.
