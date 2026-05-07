---
layout: home

hero:
  name: hyprpilot
  text: Agent-driven workflows for Linux.
  tagline: An overlay daemon that runs coding agents at the edge of your screen.
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
  - icon: ⌨️
    title: Keymap first
    details: Ctrl+K opens the palette. Sessions, profiles, models, modes, MCPs, skills, instances — all one chord away. The mouse is optional.
  - icon: 🪟
    title: Multi-instance, multi-session
    details: Run several agents at once and switch focus instantly. Resume any past session from the palette without losing the live ones.
  - icon: 🎛
    title: Pre-configured profiles
    details: Pin an agent + model + cwd + system prompt + MCPs as a profile. Spawn instances of it on demand from the palette.
  - icon: 🤖
    title: Bring your agent
    details: claude-code, codex, opencode — speak the Agent Client Protocol and you are in. Drop your existing claude.json straight in.
  - icon: 📜
    title: Skills as context
    details: Anthropic's skill convention, attached to your next prompt from the palette as a markdown resource the agent reads first.
  - icon: 🎨
    title: Themed
    details: Every color, every chip, every state — overridable from one TOML. Light + dark, gold-anchored to match the rest of your desktop.
  - icon: 📱
    title: Phone as remote
    details: Open the same overlay from any browser on the LAN. Pair once with a 4-word code or a QR scan, then drive the agent from your phone or tablet.
---
