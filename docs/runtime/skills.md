---
title: Skills & the hyprpilot MCP Server
order: 50
next: false
---

# {{ $frontmatter.title }}

Skills are `SKILL.md` bundles — reusable markdown instructions the agent can list, read, and reload. They reach the agent **only** through hyprpilot's own in-tree MCP server, which the launcher auto-injects into the vendor's MCP config.

<!-- more -->

## Skill bundles

The skills catalogue is configured under the [`mcp` block](../config/mcp#the-mcp-block); each configured root is a flat directory of `<slug>/SKILL.md` bundles, compatible with [Anthropic's skill convention](https://github.com/anthropics/skills):

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

Per-root `ignore` globs skip matching slugs at load. On a slug collision across roots, the first root wins. Missing roots warn and are skipped.

The compiled defaults seed the `mcp` block (via a root [`patches`](../config/patches) entry) with `enabled: true`, `autoAcceptTools: ['*']`, and the single XDG skills root `~/.config/hyprpilot/skills` — so skills work out of the box once you drop a `SKILL.md` in. A profile's own `mcp` block wholesale-replaces the global one — point a profile at a different skills root, or disable the server entirely.

## Auto-injection

When `mcp.enabled` is `true` **and** the resolved skills catalogue is non-empty, hyprpilot prepends a stdio MCP server named **`hyprpilot`** to the catalogue it hands the vendor. That entry launches `hyprpilot mcp serve` as a child of the agent — the vendor owns its lifetime; you never run it by hand.

- The reserved name `hyprpilot` replaces any same-named server you configured.
- Auto-inject is independent of `mcps` — `mcps: []` does not suppress it. Set `mcp.enabled: false` (or leave the skills catalogue empty) to turn it off.
- `autoAcceptTools` / `autoRejectTools` default the approval policy for the injected server; the default `['*']` accept makes skill calls frictionless.

The injected entry runs the current binary with one `--skill-dir` argument per configured root, each carrying that root's own ignore-glob list as JSON — see [the `mcp serve` reference](#hyprpilot-mcp-serve) below for the exact shape.

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

## `hyprpilot mcp serve`

The subcommand that runs the server over stdio. **You don't run this by hand** — the agent vendor spawns it as a child via the auto-injected entry.

```sh
hyprpilot mcp serve --skill-dir '{"dir":"/abs/path","ignore":[]}'
```

| Flag                 | Purpose                                                                              |
| -------------------- | ------------------------------------------------------------------------------------ |
| `--skill-dir <json>` | JSON-encoded skill root entry. Repeatable — roots are searched in declaration order. |

Each `--skill-dir` value is one self-contained JSON object:

```json
{ "dir": "/abs/path", "ignore": ["glob1", "glob2"] }
```

The launcher passes one `--skill-dir` per resolved skills root, each carrying that root's own ignore-glob list, so the sidecar rebuilds exactly the registry the launcher resolved — first-slug-wins on collision, per-root ignores applied independently.

The [global flags](./launch#global-flags) apply here too; the server owns stdin/stdout for the MCP protocol, so logs go to stderr as everywhere else.
