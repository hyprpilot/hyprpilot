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

- `hyprpilot://skills` — the **catalogue index**: every skill with its description, as one markdown document, led by a header explaining how to load them.

  Attach it (`@`-mention it, or whatever your client calls that) and it costs **no** tool call — the client injects it directly. A model _can_ also pull it where the client exposes generic resource reading (Claude Code has `ReadMcpResourceTool`), but that is still a tool call, so for the model `list_skills` remains the better route: same cost, and it is a named tool with a description to route on rather than a URI it must already know. The resource's win is the attachment path.

- `hyprpilot://skills/<slug>` — the skill body, followed by a manifest of the references it declares: each one's path and name, but not its body.

::: warning References have no URI, and that is a context-budget decision

The resource surface is the catalogue index and one entry per skill. Nothing else. Reference bodies are reached only through `load_skill_references`.

Measured against a real 127-skill catalogue: listing one entry per skill costs 128 resources and ~105 KB. Adding one bundle entry per skill took it to 231 and ~170 KB, of which 48% was `_meta` — each bundle entry repeating its own skill's block verbatim, paying twice for one skill's metadata. Enumerating all 479 individual references on top would reach **~607 entries and ~500 KB, over 120k tokens spent before a single skill is read**.

A URI would also be the wrong shape. A reference's identity is its path; a `<slug>/<name>` address is one of many addresses for one shared file, which is exactly what makes double-loading invisible.

:::

And as tools:

| Tool                    | Purpose                                                              |
| ----------------------- | -------------------------------------------------------------------- |
| `list_skills`           | Enumerate discovered skills with their metadata and reference count. |
| `read_skill`            | Fetch a skill body by slug, plus its reference manifest.             |
| `list_skill_references` | One skill's reference metadata, without bodies.                      |
| `load_skill_references` | Fetch reference bodies by path.                                      |
| `reload`                | Rescan the skill roots (picks up edits / new bundles).               |

## References

A skill declares its references in frontmatter, as paths relative to the skill's own directory:

```markdown
---
title: git-commit
description: Stage and commit changes
references:
  - ../references/commit-style.md
  - ../references/output-diff.md
---
```

### The path is the address, and the identity

`read_skill` returns the skill body plus a **manifest** — every declared reference, with the canonical path that fetches it — but not their bodies:

```jsonc
{
  "uri": "hyprpilot://skills/git-commit",
  "body": "…",
  "references": [
    {
      "path": "/home/you/.config/hyprpilot/skills/references/output-diff.md",
      "name": "output-diff",
      "size": 2481,
      "modified": "2026-08-04T09:12:33Z",
      "created": "2026-05-02T11:04:07Z"
    }
  ]
}
```

Pass those paths back to fetch bodies:

```jsonc
load_skill_references { "references": ["/…/references/output-diff.md"] }
// body plus everything, in one call
read_skill { "slug": "git-commit", "bundle": true }
```

Addressing by path rather than by skill-and-name buys three things:

- **De-duplication.** The same shared file is cited by many skills under different names. Two citations resolve to one path, so a path you already loaded needs no second fetch — and the server serves a repeated path once.
- **One call across skills.** A path names a file, not a skill, so a single call fetches references belonging to as many skills as you like.
- **No collision rules.** Paths are unique by construction, so two references sharing a label inside one skill are both fully addressable. There is nothing to shadow and no first-wins rule to remember.

Only paths that some skill actually declares are served — a caller-supplied path is checked against that set, never joined onto anything, so the surface reaches exactly the files the skills already reference. Anything else is an error rather than a partial result.

The **declared** spelling (`../references/output-diff.md`) never reaches the wire: it is meaningless outside its bundle directory, and offering it alongside the canonical path would give a caller two addresses of which only one works. Paths are canonicalized, so `..` collapses and two spellings of one file compare equal.

`list_skill_references { slug }` returns the same manifest without the skill body, for checking what a skill cites before spending tokens on it. It takes a slug rather than scanning the whole catalogue — a corpus-wide scan is a six-figure payload, and comparing paths per skill answers the same question incrementally.

Because the manifest always rides along — including as a text footer on the resource path, for clients that never surface `_meta` — declining a body is never a silent gap. The reader can always see what exists and what it has not loaded.

### Missing files and reference frontmatter

- **Missing file:** a reference that is declared but cannot be read appears in the manifest and in any bundle as a `status: not-found` marker **in its declared position**, so the gap is visible where it belongs. It has no path, so it cannot be fetched.
- **Reference frontmatter:** a reference may carry its own YAML frontmatter, parsed exactly as a skill's is. It is served with the fence stripped and its keys projected into the manifest entry's `metadata` — nothing is invented into it, because hyprpilot enforces no invocation gate and a stamped `disableModelInvocation` would imply a restriction that does not exist. A `name:` there overrides the display label.

A fetched reference carries its **full** metadata: the bundle header is built from the same manifest row the listing advertises, so the two cannot disagree.

```txt
---
reference:
  path: /home/you/.config/hyprpilot/skills/references/output-diff.md
  name: output-diff
  size: 2011
  modified: 2026-08-10T12:08:46Z
  created: 2026-08-10T10:32:30Z
---
# Output Diff
…
```

Full detail is affordable there and not in a listing: it is emitted once per reference you deliberately asked for, whereas `resources/list` pays for the whole catalogue.

### Timestamps

Skills and references both carry `size`, `modified`, and `created` as RFC 3339 UTC strings, so an agent can tell a convention it read last week from one that changed an hour ago. `created` is the filesystem birth time and is **omitted** where the platform or filesystem does not record one, rather than being back-filled from `modified` — that would answer a different question than the key names. Access time is deliberately absent: it records reads rather than writes, and lazily on the `relatime` mounts that are the Linux default.

## Frontmatter passthrough

A `SKILL.md` is markdown with an optional YAML frontmatter block. The loader keeps **every** frontmatter key losslessly, and the server passes the map through to the agent on the MCP wire so a new frontmatter field reaches the agent with zero server changes.

Metadata is carried in **one** block — never duplicated across surfaces. Per the MCP spec, `_meta` is a single field keyed by reverse-DNS names; hyprpilot emits exactly one such key and never repeats anything the spec-compliant `Resource` fields already carry:

- **Spec `Resource` fields** are canonical: `uri`, `name` (the slug), `title`, `description`, `mimeType`, `size`.
- **`io.hyprpilot/skill`** (resource `_meta`) / **`metadata`** (tool output) — the same single block: the entire frontmatter map **verbatim** (keys pass through unchanged — no camelCasing; nested maps, arrays, numbers, and booleans all convert), **minus** the keys another field already carries, **plus** the runtime-derived `path`, `bundleDir`, `size`, `modified`, and `created` (which are not in the frontmatter).

Two frontmatter keys are dropped as duplicates. `title` and `description` equal the canonical `Resource.title` / `Resource.description` byte-for-byte. `references` is superseded by the resolved [reference manifest](#references), which addresses each one by name — passing the raw array through as well would publish the declared filesystem paths, and a consumer that has those will read the files directly instead of going through the server.

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
