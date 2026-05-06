---
title: Features overview
order: 1
---

# Features

Everything the overlay does in one page.

## Supported clients

Hyprpilot speaks the [Agent Client Protocol](https://agentclientprotocol.com/) — any vendor that ships an ACP server works:

| Vendor | Status | Notes |
| --- | --- | --- |
| **Claude Code** (Anthropic) | First-class | via `@zed-industries/claude-code-acp`. Mode (`plan` / `default`), thinking budget, model picker — all wired. |
| **Codex** (OpenAI) | First-class | via `@zed-industries/codex-acp`. Approval modes propagate. |
| **opencode** | First-class | Tool filters propagate. |
| **Custom ACP** | Supported | Use `provider = "acp-custom"` with your own `command` + `args`. |

You don't see vendor-specific seams in the overlay — model picking, mode switching, permission flows look the same regardless of which agent you're talking to.

## Capabilities at a glance

- [**Command palette**](./command-palette) — `Ctrl+K` over sessions / profiles / models / modes / effort / cwd / instances / MCPs / skills / daemon.
- [**Chat & tools**](./chat-and-tools) — transcript with formatted tool pills, inline + modal permission flows, multi-instance focus.
- [**Composer**](./composer) — caret-anchored autocomplete (skills, paths, word search), drag-drop image attachments, queued submits.
- **Waybar integration** — stream the daemon's status into a Waybar module. See the [Waybar guide](../guide/waybar).
- **Tray icon** — left-click toggle, right-click menu (toggle / show / hide / shutdown).
- **Single-instance** — running `hyprpilot daemon` a second time pops the existing overlay instead of starting another daemon.
- **Layered config + theming** — one TOML defines the entire palette and chrome. Validates at boot; typos fail fast with a readable error.

## What's NOT supported

- **Persistent disk-backed trust store.** "Always allow / deny" decisions live in memory and reset on instance shutdown.
- **GNOME / KDE anchor mode.** Those compositors don't expose the layer-shell protocol. Use `mode = "center"` instead.
- **Windows / macOS layer-shell.** Layer-shell is Wayland-only — anchor mode is Linux-only by definition. Center mode works cross-platform.
- **Multi-user daemons.** One daemon per user.
