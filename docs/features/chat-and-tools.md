---
title: Chat & tools
order: 3
---

# Chat and tools

The transcript is the heart of the overlay. It's where prompts go, agent responses stream back, tool calls render as pills, and permission modals interrupt for action.

![chat transcript with bash + edit + read tool pills in a single turn](/screenshots/chat-tool-pills.png)

## Tool pills

Every tool the agent invokes shows up as a compact pill with:

- **Icon** — per-tool-family color (`read` blue, `write` magenta, `bash` orange, `search` cyan, `terminal` green, `agent` purple, `acp` light-blue).
- **Title** — formatted by the daemon (e.g. `bash: ls -la /tmp` or `edit: src/main.rs`).
- **State** — running / completed / failed / cancelled, with a per-state animation.
- **Stats** — typed mini-pills for diff sizes (`+12 −3`), durations (`850ms`, `2.4s`), match counts. Drives the "what just happened" read at a glance.

Pills are **fully formatted on the daemon side** — the UI is a dumb consumer of `FormattedToolCall` payloads. A future Neovim plugin reuses the same wire.

## Multi-instance

Run as many concurrent agents as you want. Each `(agent, profile)` pair gets a distinct UUID; spawning the same profile twice creates two independent sessions side by side.

![transcript with a pending bash permission row inline above the composer](/screenshots/permission-row.png)

Header pills (left to right): profile badge · agent · model · cwd · mode · MCP count · git status. Click any to jump to the relevant palette leaf for the focused instance.

The instances breadcrumb shows N (count); `Ctrl+K → instances` switches focus. Auto-focus rules:

- First instance to spawn auto-focuses.
- Shutting down the focused instance reassigns focus to the oldest survivor.
- Restart preserves the focused slot — the UUID is reused across the swap.

## Permission flow

When an agent requests permission for a tool — say a `bash` invocation — the request lands as a permission modal **inline in the transcript** (not as a global blocking dialog).

![plan-modal permission with markdown body and Approve / Keep planning actions](/screenshots/permission-modal.png)

Captain has four options per request:

- **Allow once** — runs this call only.
- **Allow always** — adds `(instance_id, tool_name)` to the runtime trust store; future identical requests auto-resolve.
- **Deny once** — rejects this call only.
- **Deny always** — adds the deny rule to the trust store.

**Reject beats accept** when both lanes (trust store + MCP globs) match — safer default. Vendor-native tools (Bash, Read, …) carry no `mcp__` prefix and skip the MCP lane; they only short-circuit when the captain has clicked an "always" button.

## Composer attachments

The composer accepts:

- **Plain text** — captain's prompt.
- **Image pills** — drag-drop image files from the OS, paste from clipboard via `Ctrl+P`. PNG-encoded, attached as `ContentBlock::Image`.
- **Skill pills** — `Ctrl+K → skills` picks one; the body snapshot rides on the next prompt as `ContentBlock::Resource`.

Submit (`Enter`) sends the whole compose state — text + attachments — through `session_submit` to the focused instance, or queues it if a turn is already in flight.

## Queue

When the active turn is busy, additional submits queue. The queue strip renders above the composer; `Ctrl+Enter` dispatches the next item; per-row edit / delete buttons let you reshape it before send. The queue drains automatically as turns complete.

## Status broadcast

The daemon publishes a `StatusBroadcast` (`idle` / `streaming` / `awaiting` / `error`) over `status/subscribe`. Waybar's `custom/hyprpilot` module reads this stream — see the [Waybar guide](../guide/waybar) for the drop-in.
