---
title: MCP & skills
order: 4
---

# MCP & skills

Hyprpilot resolves two related things at launch and projects them onto
the vendor CLI:

- The **MCP catalog** (`mcps`) — external Model Context Protocol servers
  the agent can call, plus a per-server tool policy.
- The **skills catalog** (`[mcp].skills`) — your `SKILL.md` bundles,
  delivered to the agent through an in-tree MCP server hyprpilot
  auto-injects.

## The MCP catalog (`mcps`)

Each `mcps` entry carries **either** a `file` path **or** an inline
`mcp_servers` map (exactly one — declaring both, or neither, is a load
error). File paths follow the standard `{ "mcpServers": { … } }` shape
that Claude Code, Codex, and Cursor all read, so you can drop your
existing `~/.claude.json` straight in.

```toml
[[profiles]]
id = "engineer"
agent = "claude-code"
mcps = [
  { file = "~/.claude.json" },
  { file = "~/.config/hyprpilot/mcps/team.json", ignore = ["scratch-*", "*-internal"] },
]
```

Inside each file:

```json
{
  "mcpServers": {
    "filesystem": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"]
    },
    "github": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-github"],
      "env": { "GITHUB_PERSONAL_ACCESS_TOKEN": "${env:GITHUB_TOKEN}" }
    }
  }
}
```

Files iterate in order; a later file's server overrides an earlier one of
the same name. A single malformed file warns and is skipped rather than
aborting the launch. Transport is inferred by field presence (`command`
→ stdio, `url` → http/sse).

### Inline servers

For one-off servers, declare `mcp_servers` on the entry directly instead
of a file:

```toml
[[profiles.mcps]]
mcp_servers = { hyprpilot-nvim = { command = "uvx", args = ["hyprpilot-nvim-mcp"] } }
```

### Ignoring servers

`ignore` is an optional glob array per entry. Server names matching any
pattern are dropped before they reach the agent. Globs anchor against the
full server name — `work-*` matches `work-foo` but not `pre-work-foo`.

### Per-profile override

`mcps` on a profile wholesale-replaces the shared catalog for that
profile. `mcps = []` means "no MCPs at all" — handy for a sandboxed
read-only profile. To share one catalog across every profile, put it in a
`[[patches]]` entry instead of repeating it — see
[Patches & overlays](./patches).

### MCP tool policy

Each server entry takes an optional `hyprpilot` block for tool
visibility and approval policy:

```json
{
  "mcpServers": {
    "filesystem": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/tmp"],
      "hyprpilot": {
        "includeTools": ["read_*", "list_*"],
        "excludeTools": ["delete_*"],
        "autoAcceptTools": ["read_*"],
        "autoRejectTools": ["delete_*"]
      }
    }
  }
}
```

- Globs are **server-relative** — write `read_*`, not
  `mcp__filesystem__read_*`; the `mcp__<server>__` prefix is implicit.
- `includeTools` / `excludeTools` control **visibility**;
  `autoAcceptTools` / `autoRejectTools` control **approval**.
- `excludeTools` wins over `includeTools`; `autoRejectTools` wins over
  `autoAcceptTools` when both match.
- `includeTools` unset means no allow-list; `includeTools = []` is an
  explicit deny-all allow-list.

The policy is projected into each vendor's native shape at launch: Claude
gets `--allowedTools` / `--disallowedTools`, OpenCode gets ordered
`OPENCODE_PERMISSION` rules, and Codex gets exact-name `enabled_tools` /
`disabled_tools` / per-tool `approval_mode`. Codex does not support
wildcard patterns in those fields, so wildcard patterns are skipped for
Codex with a warning.

## Skills and the in-tree MCP server

Skills reach the agent through hyprpilot's own MCP server. Configure the
catalog under a `[mcp]` block:

```toml
[mcp]
enabled = true                          # auto-inject the in-tree server (default true)
autoAcceptTools = ["*"]                 # default approval for the server's tools
autoRejectTools = []

[[mcp.skills]]
dir = "~/.config/hyprpilot/skills"

[[mcp.skills]]
dir = "~/.team/shared-skills"
ignore = ["work-*", "*-experimental"]
```

Each `dir` is a flat directory of `<slug>/SKILL.md` bundles, compatible
with [Anthropic's skill convention](https://github.com/anthropics/skills):

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

`ignore` is the same glob shape as `mcps` — slugs matching any pattern
are skipped at load. On a slug collision across roots, the first root
wins.

### Auto-injection

When `[mcp].enabled` is `true` **and** the resolved skills catalog is
non-empty, hyprpilot prepends a stdio MCP server named **`hyprpilot`** to
the catalog it hands the vendor. That entry launches `hyprpilot mcp
serve` as a child of the agent — the vendor owns its lifetime. Through it
the agent can list, read, and reload your skills over MCP.

- The reserved name `hyprpilot` replaces any same-named server you
  configured.
- Auto-inject is independent of `mcps` — `mcps = []` does not suppress
  it. Set `[mcp].enabled = false` (or leave the skills catalog empty) to
  turn it off.
- `autoAcceptTools` / `autoRejectTools` default the approval policy for
  the injected server; the default `["*"]` accept makes skill calls
  frictionless.

The compiled defaults seed this block (via a root `[[patches]]` entry)
with `enabled = true`, `autoAcceptTools = ["*"]`, and the single XDG
skills root, so skills work out of the box once you drop a `SKILL.md` in.

### Per-profile override

A profile's `[profiles.<id>.mcp]` block wholesale-replaces the global
`[mcp]` for that profile — point a profile at a different skills root, or
disable the server entirely.

See the [MCP server reference](../reference/mcp-server) for the tools and
resources the server exposes and how skill frontmatter is passed through
to the agent.
