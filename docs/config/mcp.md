---
title: MCP
order: 50
---

# {{ $frontmatter.title }}

Two related surfaces share this page: the **`mcps` catalogue** declares the external Model Context Protocol servers an agent can call (plus a per-server tool policy), and the **`mcp` block** configures the in-tree skills server. At launch, hyprpilot merges the catalogue and projects it onto the vendor's native MCP surface — you keep one catalogue, every vendor reads it.

<!-- more -->

## The `mcps` catalogue

Each `mcps` entry carries **either** a `file` path **or** an inline `mcp_servers` map (exactly one — declaring both, or neither, is a load error). File paths follow the standard `{ "mcpServers": { … } }` shape that Claude Code, Codex, and Cursor all read, so you can drop your existing `~/.claude.json` straight in:

```yaml
profiles:
  - id: engineer
    agent: claude-code
    mcps:
      - file: ~/.claude.json
      - file: ~/.config/hyprpilot/mcps/team.json
        ignore:
          - scratch-*
          - '*-internal'
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

Files iterate in order; a later file's server overrides an earlier one of the same name. A single malformed file warns and is skipped rather than aborting the launch. Transport is inferred by field presence (`command` → stdio, `url` → http/sse). Everything except the typed `hyprpilot` policy block stays opaque, so vendor-specific server fields pass through untouched.

### Fields

| Field         | Type             | Default | What it does                                                                   |
| ------------- | ---------------- | ------- | ------------------------------------------------------------------------------ |
| `file`        | path             | —       | An `{ "mcpServers": { … } }` JSON file. Exactly one of `file` / `mcp_servers`. |
| `mcp_servers` | map              | —       | Inline server map, same shape as the file's `mcpServers` value.                |
| `ignore`      | string[] (globs) | `[]`    | Server names matching any pattern are dropped.                                 |

### Inline servers

If you want a one-off server without a file, declare `mcp_servers` on the entry directly:

```yaml
mcps:
  - mcp_servers:
      hyprpilot-nvim:
        command: uvx
        args:
          - hyprpilot-nvim-mcp
```

### Ignoring servers

`ignore` is an optional glob array per entry. Server names matching any pattern are dropped before they reach the agent. Globs anchor against the full server name — `work-*` matches `work-foo` but not `pre-work-foo`.

### Per-profile override

`mcps` on a profile wholesale-replaces the shared catalogue for that profile. `mcps: []` means "no MCPs at all" — handy for a sandboxed read-only profile. To share one catalogue across every profile, put it in a [`patches`](./patches) entry instead of repeating it.

::: info Reserved name

The server name `hyprpilot` is reserved for the auto-injected in-tree server — see [Runtime → Skills](../runtime/skills). A configured server of that name is replaced by the injected entry.

:::

## Tool policy

Each server entry takes an optional `hyprpilot` block for tool visibility and approval policy:

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

| Field             | Type             | Default   | What it does                                                   |
| ----------------- | ---------------- | --------- | -------------------------------------------------------------- |
| `includeTools`    | string[] (globs) | unset     | Visibility allow-list. Unset = no allow-list; `[]` = deny all. |
| `excludeTools`    | string[] (globs) | `[]`      | Visibility deny-list. Exclude beats include.                   |
| `autoAcceptTools` | string[] (globs) | inherited | Approval accept list. Falls back to `mcp.autoAcceptTools`.     |
| `autoRejectTools` | string[] (globs) | inherited | Approval reject list. Reject beats accept.                     |

- Globs are **server-relative** — write `read_*`, not `mcp__filesystem__read_*`; the `mcp__<server>__` prefix is implicit.
- `includeTools` / `excludeTools` control **visibility**; `autoAcceptTools` / `autoRejectTools` control **approval**.
- Servers with no per-server override inherit the `mcp` block's `autoAcceptTools` (default `['*']`) / `autoRejectTools`.
- Every other key on a server definition passes through to the vendor untouched.

## The `mcp` block

The `mcp` block configures the in-tree skills server — what [Runtime → Skills](../runtime/skills) delivers:

```yaml
mcp:
  enabled: true
  autoAcceptTools:
    - '*'
  autoRejectTools: []
  skills:
    - dir: ~/.config/hyprpilot/skills
    - dir: ~/.team/shared-skills
      ignore:
        - work-*
        - '*-experimental'
```

| Field             | Type                 | Default  | What it does                                                                       |
| ----------------- | -------------------- | -------- | ---------------------------------------------------------------------------------- |
| `enabled`         | bool                 | `true`   | Auto-inject the in-tree `hyprpilot` server when the skills catalogue is non-empty. |
| `skills`          | `{ dir, ignore? }[]` | XDG root | Skill roots — flat directories of `<slug>/SKILL.md` bundles.                       |
| `autoAcceptTools` | string[] (globs)     | `['*']`  | Default tool-approval accept list, copied onto servers with no per-server policy.  |
| `autoRejectTools` | string[] (globs)     | `[]`     | Default tool-approval reject list. Reject beats accept.                            |

The defaults are seeded through an unscoped [`patches`](./patches) entry with the single root `~/.config/hyprpilot/skills`. A profile's `mcp` field wholesale-replaces this block.

### `skills` entries

| Field    | Type             | Default | What it does                                                               |
| -------- | ---------------- | ------- | -------------------------------------------------------------------------- |
| `dir`    | path             | —       | Skill root to scan. Missing roots warn and are skipped.                    |
| `ignore` | string[] (globs) | `[]`    | Slugs matching any pattern are skipped. First root wins on slug collision. |

## Vendor projection

The merged catalogue and policy are projected into each vendor's native shape at launch:

| Vendor        | Servers via                            | Policy via                                                      |
| ------------- | -------------------------------------- | --------------------------------------------------------------- |
| `claude-code` | `--mcp-config <path>` (0600 temp file) | `--allowedTools` / `--disallowedTools` (`mcp__server__tool`)    |
| `codex`       | `-c mcp_servers.<name>.*` overrides    | exact-name `enabled_tools` / `disabled_tools` / `approval_mode` |
| `opencode`    | `OPENCODE_CONFIG_CONTENT` env          | ordered `OPENCODE_PERMISSION` rules (`server_tool`)             |

Codex does not support wildcard tool patterns in those fields, so wildcard patterns are skipped for Codex with a warning. Provider-native arguments you pass after `--` (or env you set on the agent) suppress the generated equivalents.

## Secrets in the vendor handoff

MCP server entries commonly carry secrets — bearer tokens in HTTP `headers`, API keys in stdio `env`. hyprpilot keeps expanded secret material out of the vendor's **argv**, because a process's argv is world-readable through `/proc/<pid>/cmdline` on Linux:

- **`claude-code`** — the resolved MCP config (with `${VAR}` header/env references already expanded) is written to a per-launch **0600 temp file** and passed as `--mcp-config <path>`. The file is created owner-only from the start, so the secret never lands in argv. hyprpilot `exec()`s into the vendor and does not delete the file first — the vendor needs to read it after the handoff; it is a launch-scoped temp the OS reclaims on tmp cleanup.
- **`codex`** — bearer tokens are projected as `mcp_servers.<name>.bearer_token_env_var` / `env_http_headers` references (the env var name, not its value), so codex resolves the secret from its own environment.
- **`opencode`** — the generated config rides the `OPENCODE_CONFIG_CONTENT` env var. Env is not world-readable like argv, but does inherit into child processes; this is the residual, lower-risk surface.
