---
title: Extensions
order: 5
---

# Extensions: MCPs and Skills

Both ride on captain-supplied file paths in TOML — drop in JSON / Markdown, the daemon picks them up at boot.

## MCPs

```toml
mcps = [
  "~/.config/hyprpilot/mcps/team.json",
  "~/.claude.json"
]
```

Each path follows the standard `mcpServers` JSON shape used by Claude Code, Codex, Cursor — **drop `~/.claude.json` straight in and it works**.

```json
{
  "mcpServers": {
    "filesystem": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
      "hyprpilot": {
        "autoAcceptTools": ["read_*"],
        "autoRejectTools": ["delete_*"]
      }
    }
  }
}
```

### Hyprpilot extension fields

The optional `hyprpilot` namespace per server entry:

| Field | Purpose |
| --- | --- |
| `autoAcceptTools: string[]` | Glob list. Tool calls matching auto-resolve to `Allow` without surfacing a permission prompt. |
| `autoRejectTools: string[]` | Glob list. Tool calls matching auto-resolve to `Deny`. **Reject beats accept** when both match. |

Globs are **server-relative** — write `read_*` (not `mcp__filesystem__read_*`). The `mcp__<server>__` prefix is implicit.

### Merge semantics

- Files iterate in order; map collisions on the same server name = later wins.
- Per-profile override: `[[profiles]] mcps = [...]` wholesale-replaces the global default. `mcps = []` is the explicit off-switch.
- Static after boot. Edit the JSON, restart the daemon. (ACP fixes `mcpServers` at `session/new`, so a reload would only land for new instances anyway.)

### Permission flow

1. Tool request arrives via `session/request_permission`.
2. Daemon checks the runtime trust store (UI's "always allow / always deny" buttons) — if hit, short-circuit.
3. Daemon checks the MCP `hyprpilot.{auto,reject}Tools` globs — if hit, short-circuit.
4. Otherwise, surface the prompt to the captain via the permission stack.

## Skills

```toml
[skills]
dirs = ["~/.config/hyprpilot/skills"]
```

Each root is a flat directory of `<slug>/SKILL.md` bundles — compatible with the [claude-code skill convention](https://github.com/anthropics/claude-code/blob/main/skills/README.md).

```
~/.config/hyprpilot/skills/
├── git-commit/
│   └── SKILL.md
├── linear-issue/
│   ├── SKILL.md
│   └── references/
└── github-pr/
    └── SKILL.md
```

### Behavior

- **Multi-root.** List multiple `dirs`; first-root-wins on slug collision (`warn!` logged).
- **Hot-reload.** A `notify` watcher rescans on add / modify / remove. No daemon restart needed.
- **Missing roots** warn + skip (no auto-mkdir). Recovery: `hyprpilot ctl skills reload` after creating the directory.

### Delivery to the agent

Skills attach to user turns through the **palette**, not inline tokens. Captain opens `Ctrl+K → skills`, picks one, the body snapshot rides on the next prompt as an embedded resource. The agent reads context first, then the user's instructions.

The old inline `#{skill/<slug>}` token mechanism was removed — palette is the single delivery surface.
