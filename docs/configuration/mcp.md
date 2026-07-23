---
title: '[mcp] & mcps'
order: 30
---

# {{ $frontmatter.title }}

Two related surfaces: the `[mcp]` block configures the in-tree skills server, `mcps` lists external MCP servers. Narratives: [Features → MCP Catalogue](../features/mcp) and [Features → Skills](../features/skills).

<!-- more -->

## `[mcp]`

```toml
[mcp]
enabled = true
autoAcceptTools = ["*"]
autoRejectTools = []

[[mcp.skills]]
dir = "~/.config/hyprpilot/skills"
ignore = []
```

| Field             | Type                 | Default  | What it does                                                                       |
| ----------------- | -------------------- | -------- | ---------------------------------------------------------------------------------- |
| `enabled`         | bool                 | `true`   | Auto-inject the in-tree `hyprpilot` server when the skills catalogue is non-empty. |
| `skills`          | `{ dir, ignore? }[]` | XDG root | Skill roots — flat directories of `<slug>/SKILL.md` bundles.                       |
| `autoAcceptTools` | string[] (globs)     | `["*"]`  | Default tool-approval accept list, copied onto servers with no per-server policy.  |
| `autoRejectTools` | string[] (globs)     | `[]`     | Default tool-approval reject list. Reject beats accept.                            |

The defaults are seeded through an unscoped `[[patches]]` entry (see [`[[patches]]`](./patches)) with the single root `~/.config/hyprpilot/skills`. A profile's `mcp` field wholesale-replaces this block.

### `skills` entries

| Field    | Type             | Default | What it does                                                               |
| -------- | ---------------- | ------- | -------------------------------------------------------------------------- |
| `dir`    | path             | —       | Skill root to scan. Missing roots warn and are skipped.                    |
| `ignore` | string[] (globs) | `[]`    | Slugs matching any pattern are skipped. First root wins on slug collision. |

## `mcps`

A per-profile (or patch-supplied) list of catalogue entries:

```toml
mcps = [
  { file = "~/.claude.json" },
  { file = "~/.config/hyprpilot/mcps/team.json", ignore = ["scratch-*"] },
  { mcp_servers = { time = { command = "uvx", args = ["mcp-server-time"] } } },
]
```

| Field         | Type             | Default | What it does                                                                   |
| ------------- | ---------------- | ------- | ------------------------------------------------------------------------------ |
| `file`        | path             | —       | An `{ "mcpServers": { … } }` JSON file. Exactly one of `file` / `mcp_servers`. |
| `mcp_servers` | map              | —       | Inline server map, same shape as the file's `mcpServers` value.                |
| `ignore`      | string[] (globs) | `[]`    | Server names matching any pattern are dropped.                                 |

Entries iterate in order; later wins on server-name collision. `mcps = []` is the explicit off-switch. The server name `hyprpilot` is reserved for the auto-injected entry.

## The per-server `hyprpilot` block

Inside a server definition (file or inline), an optional `hyprpilot` key carries the tool policy:

| Field             | Type             | Default   | What it does                                                   |
| ----------------- | ---------------- | --------- | -------------------------------------------------------------- |
| `includeTools`    | string[] (globs) | unset     | Visibility allow-list. Unset = no allow-list; `[]` = deny all. |
| `excludeTools`    | string[] (globs) | `[]`      | Visibility deny-list. Exclude beats include.                   |
| `autoAcceptTools` | string[] (globs) | inherited | Approval accept list. Falls back to `[mcp].autoAcceptTools`.   |
| `autoRejectTools` | string[] (globs) | inherited | Approval reject list. Reject beats accept.                     |

Globs are server-relative (`read_*`, not `mcp__server__read_*`). Every other key on a server definition passes through to the vendor untouched.
