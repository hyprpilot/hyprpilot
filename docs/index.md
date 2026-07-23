---
layout: home

hero:
  name: hyprpilot
  text: Launch your coding agent, pre-configured.
  tagline: A config-driven, fire-and-exec launcher for terminal coding agents — resolve a profile, project it onto the vendor's native CLI, and get out of the way.
  image:
    src: /icon.png
    alt: hyprpilot
  actions:
    - theme: brand
      text: Install
      link: /guide/installation
    - theme: brand
      text: Quickstart
      link: /guide/quickstart
    - theme: alt
      text: GitHub
      link: https://github.com/hyprpilot/hyprpilot
    - theme: alt
      text: AUR
      link: https://aur.archlinux.org/packages/hyprpilot-bin

features:
  - icon: 🎛
    title: Profiles
    details: Pin an agent + model + cwd + system prompt + MCPs as a session profile. `hyprpilot -p engineer` launches it; bare `hyprpilot` opens an interactive picker.
    link: /features/profiles
  - icon: 🚀
    title: Native exec, no daemon
    details: One `exec()` into the vendor's own TUI — claude, codex, or opencode. No background process, no socket, no window in between.
    link: /guide/what-is
  - icon: 🔌
    title: MCP catalogue & tool policy
    details: Drop your existing `mcpServers` JSON straight in, then gate visibility and approval per tool with server-relative globs.
    link: /features/mcp
  - icon: 📜
    title: Skills over MCP
    details: Your SKILL.md catalogue reaches the agent through an in-tree MCP server hyprpilot auto-injects — frontmatter passed through losslessly.
    link: /features/skills
  - icon: 🧩
    title: Patches & overlays
    details: Share knobs across profiles with `[[patches]]`, or fold a one-off overlay into a single launch with `--with-config`.
    link: /features/patches
  - icon: 🪟
    title: At home in your multiplexer
    details: Renames the current tmux window / zellij tab to `hyprpilot@<cwd>` right before exec, so agent panes are easy to tell apart.
    link: /features/multiplexer
---

<div style="display: flex; gap: 0.5rem; flex-wrap: wrap; justify-content: center; margin-top: 2rem;">

[![CI](https://img.shields.io/github/actions/workflow/status/hyprpilot/hyprpilot/ci.yml?label=ci&style=flat-square&logo=github)](https://github.com/hyprpilot/hyprpilot/actions/workflows/ci.yml)
[![release](https://img.shields.io/github/v/release/hyprpilot/hyprpilot?style=flat-square&logo=github)](https://github.com/hyprpilot/hyprpilot/releases)
[![AUR](https://img.shields.io/aur/version/hyprpilot-bin?style=flat-square&logo=archlinux)](https://aur.archlinux.org/packages/hyprpilot-bin)
[![license](https://img.shields.io/github/license/hyprpilot/hyprpilot?style=flat-square)](https://github.com/hyprpilot/hyprpilot/blob/main/LICENSE)

</div>
