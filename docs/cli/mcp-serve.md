---
title: hyprpilot mcp serve
order: 30
next: false
---

# {{ $frontmatter.title }}

Runs the in-tree MCP server over stdio. **You don't run this by hand** — the agent vendor spawns it as a child when hyprpilot auto-injects the `hyprpilot` MCP entry. What the server exposes (resources, tools, frontmatter passthrough) is documented at [Features → Skills](../features/skills).

<!-- more -->

```sh
hyprpilot mcp serve --skill-dir '{"dir":"/abs/path","ignore":[]}'
```

## Flags

| Flag                 | Purpose                                                                              |
| -------------------- | ------------------------------------------------------------------------------------ |
| `--skill-dir <json>` | JSON-encoded skill root entry. Repeatable — roots are searched in declaration order. |

Each `--skill-dir` value is one self-contained JSON object:

```json
{ "dir": "/abs/path", "ignore": ["glob1", "glob2"] }
```

The launcher passes one `--skill-dir` per resolved skills root, each carrying that root's own ignore-glob list, so the sidecar rebuilds exactly the registry the launcher resolved — first-slug-wins on collision, per-root ignores applied independently.

The [global flags](./#global-flags) apply here too; the server owns stdin/stdout for the MCP protocol, so logs go to stderr as everywhere else.
