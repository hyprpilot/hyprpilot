---
title: Skills & the hyprpilot MCP Server
order: 60
---

# {{ $frontmatter.title }}

Skills are `SKILL.md` bundles — reusable markdown instructions the agent can list, read, and reload. They reach the agent **only** through hyprpilot's own in-tree MCP server, which the launcher auto-injects into the vendor's MCP config.

<!-- more -->

## The `[mcp]` block

Configure the skills catalogue under a `[mcp]` block:

```toml
[mcp]
enabled = true # auto-inject the in-tree server (default true)
autoAcceptTools = ["*"] # default approval for the server's tools
autoRejectTools = []

[[mcp.skills]]
dir = "~/.config/hyprpilot/skills"

[[mcp.skills]]
dir = "~/.team/shared-skills"
ignore = ["work-*", "*-experimental"]
```

Each `dir` is a flat directory of `<slug>/SKILL.md` bundles, compatible with [Anthropic's skill convention](https://github.com/anthropics/skills):

```txt
~/.config/hyprpilot/skills/
├── git-commit/
│   └── SKILL.md
├── linear-issue/
│   ├── SKILL.md
│   └── references/
└── github-pr/
    └── SKILL.md
```

`ignore` is the same glob shape as `mcps` — slugs matching any pattern are skipped at load. On a slug collision across roots, the first root wins. Missing roots warn and are skipped.

The compiled defaults seed this block (via a root [`[[patches]]`](./patches) entry) with `enabled = true`, `autoAcceptTools = ["*"]`, and the single XDG skills root, so skills work out of the box once you drop a `SKILL.md` in. A profile's own `mcp` block wholesale-replaces the global one — point a profile at a different skills root, or disable the server entirely.

## Auto-injection

When `[mcp].enabled` is `true` **and** the resolved skills catalogue is non-empty, hyprpilot prepends a stdio MCP server named **`hyprpilot`** to the catalogue it hands the vendor. That entry launches `hyprpilot mcp serve` as a child of the agent — the vendor owns its lifetime; you never run it by hand.

- The reserved name `hyprpilot` replaces any same-named server you configured.
- Auto-inject is independent of `mcps` — `mcps = []` does not suppress it. Set `[mcp].enabled = false` (or leave the skills catalogue empty) to turn it off.
- `autoAcceptTools` / `autoRejectTools` default the approval policy for the injected server; the default `["*"]` accept makes skill calls frictionless.

The injected entry runs the current binary with one `--skill-dir` argument per configured root, each carrying that root's own ignore-glob list as JSON — see the [CLI reference](../cli/mcp-serve) for the exact shape.

## What the server exposes

`hyprpilot mcp serve` is a small [rmcp](https://github.com/modelcontextprotocol/rust-sdk) stdio server.

Skills are exposed as MCP resources:

- `hyprpilot://skills/<slug>` — the skill body.
- `hyprpilot://skills/<slug>/references` — the bundle's declared reference files, resolved relative to the skill directory.

And as tools:

| Tool                    | Purpose                                                   |
| ----------------------- | --------------------------------------------------------- |
| `list_skills`           | Enumerate discovered skills with their metadata.          |
| `read_skill`            | Fetch a skill body by slug.                               |
| `load_skill_references` | Bundle the reference files a skill declares.              |
| `reload`                | Rescan the skill roots (picks up edits / new bundles).    |
| `open`                  | Open a URL, file, or directory in the OS default handler. |

Skills are discovered by directory scan — the same discovery the launcher uses — so editing a skill and calling `reload` refreshes the catalogue without restarting the agent session.

## Frontmatter passthrough

A `SKILL.md` is markdown with an optional YAML frontmatter block. The loader keeps **every** frontmatter key losslessly, and the server passes the whole map through to the agent on the MCP wire under the resource's `_meta`, so a new frontmatter field reaches the agent with zero server changes:

- `io.hyprpilot/frontmatter` — the entire frontmatter map, verbatim. Keys pass through unchanged (no camelCasing); nested maps, arrays, numbers, and booleans all convert. The consumer interprets — renaming is interpretation the server deliberately doesn't do.
- `io.hyprpilot/skill` — a curated / derived view (name, interaction, argument hint, references, path, bundle dir) for consumers that want the pre-derived shape without re-deriving it themselves.

Both keys are namespaced per the MCP spec's `_meta` reverse-DNS convention. Frontmatter that isn't map-shaped, or a `SKILL.md` with no frontmatter fence at all, is treated as an empty map — a malformed block never fails the request.

::: details Example — every key reaches the agent

This `SKILL.md`:

```markdown
---
name: plan-hard
disable-model-invocation: true
metadata:
  owner: captain
  tags: [alpha, beta]
---

# Plan hard

…skill body…
```

…reaches the agent with every one of those keys (`name`, `disable-model-invocation`, the nested `metadata` map) intact under `io.hyprpilot/frontmatter`.

:::
