---
layout: home

hero:
  name: hyprpilot
  text: Agent-driven workflows for Hyprland.
  tagline: A Tauri overlay daemon that runs ACP-speaking coding agents at the edge of your screen.
  image:
    src: /icon.png
    alt: hyprpilot
  actions:
    - theme: brand
      text: Install
      link: /guide/installation
    - theme: alt
      text: Features
      link: /features/
    - theme: alt
      text: GitHub
      link: https://github.com/hyprpilot/hyprpilot

features:
  - icon: ⚡
    title: Native overlay
    details: zwlr_layer_shell anchor on Hyprland & Sway, regular top-level window everywhere else. One config knob switches modes.
  - icon: 🤖
    title: Multi-agent
    details: claude-code, codex, opencode — all wired through the Agent Client Protocol. Run multiple instances side-by-side.
  - icon: 🎯
    title: Captain-driven
    details: Ctrl+K palette over sessions, profiles, models, modes, MCPs, skills, instances. Keymap-first, mouse-optional.
  - icon: 🔌
    title: MCP-native
    details: Drop ~/.claude.json straight in. Per-server auto-accept / auto-reject globs for safe automation.
  - icon: 📜
    title: Skills as first-class
    details: Anthropic's claude-code skill convention, attached to user turns as embedded resources at palette-pick time.
  - icon: 🎨
    title: Themed in Rust
    details: One TOML defines the entire palette. The webview reads it; CSS doesn't redeclare anything.
---
