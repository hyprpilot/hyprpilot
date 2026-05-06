---
title: Configuration overview
order: 1
---

# Configuration

Hyprpilot reads layered TOML. Sources resolve in this order; later layers override earlier ones for the fields they set:

1. **Compiled defaults** — `src-tauri/src/config/defaults.toml`, embedded in the binary. The single source of truth for default values.
2. **Global config** — `$XDG_CONFIG_HOME/hyprpilot/config.toml` (typically `~/.config/hyprpilot/config.toml`).
3. **Per-profile config-layer** — `~/.config/hyprpilot/profiles/<name>.toml` when `--config-profile <name>` or `HYPRPILOT_CONFIG_PROFILE=<name>` is set.
4. **CLI flags** — overrides per-invocation, never persisted.

## Two `profile` concepts

The word "profile" lives in two parallel namespaces:

| Concept | Where | Purpose |
| --- | --- | --- |
| Config-layer profile | `--config-profile` CLI flag | Pick a different config TOML overlay (e.g. `work` vs `personal`). |
| Session profile | `[[profiles]]` blocks in TOML | Pick which agent + model + cwd + system prompt a chat instance uses. |

This page is about the TOML structure. For session profiles, see [Agents](./agents).

## Where things live

- **User config:** `~/.config/hyprpilot/config.toml`
- **MCP catalog:** captain-supplied JSON paths listed under top-level `mcps = […]`
- **Skills directories:** captain-supplied paths under `[skills] dirs = […]`
- **State + logs:** `$XDG_STATE_HOME/hyprpilot/logs/hyprpilot.log.<date>`
- **Socket:** `$XDG_RUNTIME_DIR/hyprpilot.sock`

## Validation

Every section uses `deny_unknown_fields` — typos in user TOML reject at load time with a readable error. Cross-field constraints (e.g. `agent.default` must reference a real `[[agents]].id`) trip the same path.

A deliberately broken `config.toml` aborts startup naming the offending field. Captain doesn't ship to a half-configured daemon.

## What's in the rest of this section

- [Window](./window) — anchor vs center mode, monitor selection, sizing.
- [Theme](./theme) — palette tokens (everything in `[ui.theme.*]`).
- [Agents](./agents) — `[agent]`, `[[agents]]`, `[[profiles]]` registries.
- [Extensions](./extensions) — MCPs and skills.
