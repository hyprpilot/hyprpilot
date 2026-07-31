---
title: Skills & the hyprpilot MCP Server
order: 50
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

The compiled defaults seed the XDG skills root `~/.config/hyprpilot/skills` (via a root [`patches`](../config/patches) entry), and the built-in `mcp` defaults (`enabled: true`, `autoAcceptTools: ['*']`) fill in the rest — so skills work out of the box once you drop a `SKILL.md` in. A profile's own `mcp` block wholesale-replaces the global one — point a profile at a different skills root, or disable the server entirely.

## Auto-injection

When `mcp.enabled` is `true`, `mcp.skills.enabled` is `true` (the default), **and** the resolved skills catalogue is non-empty, hyprpilot prepends a stdio MCP server named **`hyprpilot_skills`** to the catalogue it hands the vendor. That entry launches `hyprpilot mcp skills` as a child of the agent — the vendor owns its lifetime; you never run it by hand.

- The reserved name replaces any same-named server you configured. Rename it with `mcp.skills.name`.
- Auto-inject is independent of `mcps` — `mcps: []` does not suppress it. Set `mcp.skills.enabled: false` (this server only), `mcp.enabled: false` (all three in-tree servers), or leave the skills catalogue empty to turn it off.
- This is the one server also gated on **content**: no discovered skills means nothing is injected, since there would be nothing to serve.
- `autoAcceptTools` / `autoRejectTools` default the approval policy for the injected server; the default `['*']` accept makes skill calls frictionless.

The injected entry runs the current binary with one `--skill-dir` argument per configured root, each carrying that root's own ignore-glob list as JSON — see [the `mcp skills` reference](#hyprpilot-mcp-skills) below for the exact shape.

## What the server exposes

`hyprpilot mcp skills` is a small [rmcp](https://github.com/modelcontextprotocol/rust-sdk) stdio server.

Skills are exposed as MCP resources:

- `hyprpilot://skills/<slug>` — the skill body.
- `hyprpilot://references/<slug>` — the bundle's declared reference files, resolved relative to the skill directory. This is a parallel top-level scheme, not a `/references` segment nested under the slug — the nested form broke client URI autocomplete.

And as tools:

| Tool                    | Purpose                                                |
| ----------------------- | ------------------------------------------------------ |
| `list_skills`           | Enumerate discovered skills with their metadata.       |
| `read_skill`            | Fetch a skill body by slug.                            |
| `load_skill_references` | Bundle the reference files a skill declares.           |
| `reload`                | Rescan the skill roots (picks up edits / new bundles). |

Skills are discovered by directory scan — the same discovery the launcher uses — so editing a skill and calling `reload` refreshes the catalogue without restarting the agent session. Because each skill is exposed as a resource, `reload` also emits a **resource list-changed notification** so a connected client re-fetches the skill list instead of trusting a stale one. (The tool list is static, so no tool-list-changed fires.)

That is the whole skills surface. The other two in-tree servers are separate processes with separate catalogue entries: `hyprpilot mcp serve` carries `open` (see [the `mcp` block](../config/mcp#mcp-serve)), and `hyprpilot mcp harness` carries `list_profiles` / `spawn` / `session_*` for launching and driving other hyprpilot sessions — off by default, documented in [Agent Harness](./harness).

Every tool result carries **both** a human-readable text block and the structured JSON payload. Clients that render only structured content (Claude Code) read the JSON; clients that render only text (opencode) get a legible summary — e.g. `list_skills` returns a one-line-per-skill catalogue as text alongside the full structured list, and `read_skill` returns the skill body as text alongside the structured `{ uri, body, metadata }`. A structured-only result would otherwise render as "Unknown" in text-only clients.

## Frontmatter passthrough

A `SKILL.md` is markdown with an optional YAML frontmatter block. The loader keeps **every** frontmatter key losslessly, and the server passes the map through to the agent on the MCP wire so a new frontmatter field reaches the agent with zero server changes.

Metadata is carried in **one** block — never duplicated across surfaces. Per the MCP spec, `_meta` is a single field keyed by reverse-DNS names; hyprpilot emits exactly one such key and never repeats anything the spec-compliant `Resource` fields already carry:

- **Spec `Resource` fields** are canonical: `uri`, `name` (the slug), `title`, `description`, `mimeType`, `size`.
- **`io.hyprpilot/skill`** (resource `_meta`) / **`metadata`** (tool output) — the same single block: the entire frontmatter map **verbatim** (keys pass through unchanged — no camelCasing; nested maps, arrays, numbers, and booleans all convert), **minus** `title` and `description` (those equal the canonical `Resource.title` / `Resource.description` byte-for-byte), **plus** the runtime-derived `path` and `bundleDir` (which are not in the frontmatter).

Frontmatter `name` is **kept** in the block — `Resource.name` is the slug, while a frontmatter `name` is an author-supplied value that may differ, so it is not a spec duplicate. Frontmatter that isn't map-shaped, or a `SKILL.md` with no frontmatter fence at all, is treated as an empty map — a malformed block never fails the request.

::: details Example — every key reaches the agent

This `SKILL.md`:

```markdown
---
name: plan-hard
title: Plan hard
description: Deep planning
disable-model-invocation: true
metadata:
  owner: captain
  tags: [alpha, beta]
---

# Plan hard

…skill body…
```

…reaches the agent with `title` / `description` on the spec `Resource` fields, and every other key (`name`, `disable-model-invocation`, the nested `metadata` map) plus the runtime `path` / `bundleDir` intact under the single `io.hyprpilot/skill` block — `title` and `description` are **not** repeated inside it.

:::

## `hyprpilot mcp skills`

The subcommand that runs the server over stdio. **You don't run this by hand** — the agent vendor spawns it as a child via the auto-injected entry.

```sh
hyprpilot mcp skills --skill-dir '{"dir":"/abs/path","ignore":[]}'
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
