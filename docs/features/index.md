---
title: Features overview
order: 1
---

# Features

Everything the overlay does at a glance.

## Supported agents

Hyprpilot speaks the [Agent Client Protocol](https://agentclientprotocol.com/) — any vendor that ships an ACP server works:

| Vendor | Notes |
| --- | --- |
| **Claude Code** (Anthropic) | First-class. Mode (`plan` / `default`), thinking budget, model picker — all wired. |
| **Codex** (OpenAI) | First-class. Approval modes propagate. |
| **opencode** | First-class. Tool filters propagate. |
| **Custom** | Any binary speaking ACP — supply your own `command` + `args` under `provider = "acp-custom"`. |

You don't see vendor seams in the overlay — model picking, mode switching, permission flows look the same regardless of which agent you're talking to.

## Capabilities

- [**Command palette**](./command-palette) — `Ctrl+K` over sessions, profiles, models, modes, effort, cwd, instances, MCPs, skills, daemon ops.
- [**Chat & tools**](./chat-and-tools) — transcript with tool pills, inline + modal permission flows, multi-instance focus.
- [**Composer**](./composer) — caret-anchored autocomplete, drag-drop image attachments, queued submits.
- [**Daemon & CLI**](./cli) — every action the palette can do, scriptable from the shell. Push-to-talk, status streaming, daemon lifecycle.
- [**Remote bridge**](./remote) — open the same overlay on your phone or any other browser on the LAN. Pair once, drive remotely.
- **Pre-configured profiles** — pin agent + model + cwd + system prompt + MCPs as a profile, spawn instances of it from the palette. See [Configuration → Profiles](../configuration/profiles).
- **MCP-native** — drop your existing `~/.claude.json` straight in. Per-server auto-accept / auto-reject globs short-circuit prompts you've already decided.
- **Skills as context** — Anthropic's claude-code skill convention. Pick a skill from the palette; its body rides on your next prompt as a markdown resource.
- **Waybar integration** — stream the daemon's status into a Waybar module. See the [Waybar guide](../guide/waybar).
- **Tray icon** — left-click toggle, right-click menu (toggle / show / hide / shutdown).
- **Single-instance** — running `hyprpilot daemon` a second time pops the existing overlay instead of starting another daemon.
- **Layered config + theming** — one TOML defines the entire palette and chrome. Validates at boot; typos fail fast with a readable error.

## What's not supported

- **Persistent disk-backed trust store.** "Always allow / always deny" decisions live in memory and reset on instance shutdown.
- **GNOME / KDE anchor mode.** Those compositors don't expose the necessary protocol. Use `mode = "center"` instead.
- **Windows / macOS anchor mode.** Anchor mode is Linux-only by definition. Center mode works cross-platform.
- **Multi-user daemons.** One daemon per user.
