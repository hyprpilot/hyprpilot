---
layout: home

hero:
  name: hyprpilot
  text: Launch your coding agent, your way.
  tagline: A config-driven launcher that resolves a profile and execs the vendor's native CLI — claude, codex, or opencode.
  image:
    src: /icon.png
    alt: hyprpilot
  actions:
    - theme: brand
      text: Install
      link: /guide/installation
    - theme: alt
      text: Configuration
      link: /configuration/
    - theme: alt
      text: GitHub
      link: https://github.com/hyprpilot/hyprpilot

features:
  - icon: 🚀
    title: Fire and exec
    details: One `exec()` into the vendor's native TUI — no background daemon, no socket, no window. Hyprpilot resolves your profile then gets out of the way.
  - icon: 🎛
    title: Pre-configured profiles
    details: Pin an agent + model + cwd + system prompt + MCPs as a profile. `hyprpilot -p engineer` launches it; bare `hyprpilot` opens an interactive picker.
  - icon: 🤖
    title: Bring your agent
    details: claude-code, codex, opencode — each projected onto its own native flags. Drop your existing MCP JSON straight in.
  - icon: 🧩
    title: Layered config
    details: Compiled defaults → global TOML → per-profile overlay → `[[patches]]` → `--with-config`. Write only what you want to change.
  - icon: 📜
    title: Skills over MCP
    details: Your SKILL.md catalogue reaches the agent through an in-tree MCP server hyprpilot auto-injects — full frontmatter passed through as `_meta`.
  - icon: 🪟
    title: Multiplexer aware
    details: Renames the current tmux window / zellij tab to `hyprpilot@<cwd>` right before exec, so agent panes are easy to tell apart.
---
