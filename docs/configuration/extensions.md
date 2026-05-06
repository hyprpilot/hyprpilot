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

- Files iterate in order; collisions on the same server name = later wins.
- Per-profile override: `[[profiles]] mcps = [...]` wholesale-replaces the global default. `mcps = []` is the explicit off-switch.
- Static after boot. Edit the JSON, restart the daemon.

### Permission flow

1. Tool request arrives.
2. The runtime trust store ("always allow / always deny" from the UI) is consulted first.
3. The MCP auto-accept / auto-reject globs are consulted next.
4. Otherwise, the prompt surfaces in the permission stack.

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

- **Multi-root.** List multiple `dirs`; first root wins on slug collision.
- **Hot-reload.** Add, edit, or remove files — the daemon picks up changes without restart.
- **Missing roots** are warned about and skipped. Run `hyprpilot ctl skills reload` after creating a missing directory.

### Delivery to the agent

Skills attach to user turns through the **palette**. Open `Ctrl+K → skills`, pick one, and its body rides on your next prompt as context. The agent reads the skill first, then your message.
