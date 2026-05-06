---
title: Configuration overview
order: 1
---

# Configuration

Hyprpilot reads layered TOML. Each source overrides the one before it, so you only need to write what you want to change:

1. **Built-in defaults** — embedded in the binary. Everything has a working default.
2. **Global config** — `~/.config/hyprpilot/config.toml`.
3. **Per-profile overlay** — `~/.config/hyprpilot/profiles/<name>.toml`, picked with `--config-profile <name>` or `HYPRPILOT_CONFIG_PROFILE=<name>`.
4. **CLI flags** — override-per-invocation, never persisted.

## Two `profile` concepts

The word "profile" lives in two parallel namespaces:

| Concept | Where | Purpose |
| --- | --- | --- |
| Config-layer profile | `--config-profile` CLI flag | Pick a different config TOML overlay (e.g. `work` vs `personal`). |
| Session profile | `[[profiles]]` blocks in TOML | Pick which agent + model + cwd + system prompt a chat instance uses. |

This page is about the TOML structure. For session profiles, see [Agents](./agents).

## Where things live

- **User config:** `~/.config/hyprpilot/config.toml`
- **MCPs:** JSON paths listed under top-level `mcps = […]`
- **Skills:** directories listed under `[skills] dirs = […]`
- **Logs:** `~/.local/state/hyprpilot/logs/hyprpilot.log.*`
- **Socket:** `$XDG_RUNTIME_DIR/hyprpilot.sock`

## Validation

Every section rejects unknown fields and validates types at boot — typos in your TOML fail fast with a readable error naming the offending field. Cross-field references (e.g. `agent.default` must reference a real `[[agents]].id`) are checked too.

## What's in the rest of this section

- [Window](./window) — anchor vs center mode, monitor selection, sizing.
- [Theme](./theme) — every color in the overlay.
- [Agents](./agents) — `[agent]`, `[[agents]]`, `[[profiles]]` registries.
- [Extensions](./extensions) — MCPs and skills.
