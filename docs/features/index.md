---
title: Features overview
order: 1
---

# Features

Everything the overlay does in one page. Click through for detail.

## Supported clients

Hyprpilot speaks the [Agent Client Protocol](https://agentclientprotocol.com/) — any vendor that ships an ACP server works:

| Vendor | Status | Notes |
| --- | --- | --- |
| **Claude Code** (Anthropic) | First-class | via `@zed-industries/claude-code-acp`. Mode (`plan` / `default`), thinking budget, model picker — all wired. |
| **Codex** (OpenAI) | First-class | via `@zed-industries/codex-acp`. Approval modes propagate. |
| **opencode** | First-class | Tool filters propagate. |
| **Custom ACP** | Supported | Use `provider = "acp-custom"` with your own `command` + `args`. |

Every wire surface goes through the same `AcpAdapter` — captain doesn't see vendor-specific seams.

## Capabilities at a glance

- [**Command palette**](./command-palette) — `Ctrl+K` over sessions / profiles / models / modes / effort / cwd / instances / MCPs / skills / daemon.
- [**Chat & tools**](./chat-and-tools) — transcript with formatted tool pills, permission modals, multi-instance focus.
- [**Composer**](./composer) — caret-anchored autocomplete (skills, paths, ripgrep), drag-drop image attachments, queued submits.
- **Waybar integration** — `hyprpilot ctl status --watch` streams JSON state for waybar's custom modules. See [Waybar guide](../guide/waybar).
- **Tray icon** — left-click toggle, right-click menu (toggle / show / hide / shutdown).
- **Single-instance** — second `hyprpilot daemon` invocation forwards through to the running daemon and exits 0.
- **Layered config + theming** — Rust-owned palette, TOML-defined, deny-unknown-fields validation.

## What's NOT supported

These are intentionally out of scope; check the [GitHub issues](https://github.com/hyprpilot/hyprpilot/issues) for tracking:

- **Persistent disk-backed trust store.** "Always allow / deny" decisions are in-memory, reset on instance shutdown. Disk persistence pending.
- **GNOME / KDE anchor mode.** Those compositors don't expose `zwlr_layer_shell_v1`. Use `mode = "center"` instead.
- **Windows / macOS layer-shell.** Layer-shell is Wayland-only. Anchor mode is Linux-only by definition; center mode is cross-platform.
- **Multi-user daemons.** One daemon per user; no auth on the unix socket.
