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

- `hyprpilot://skills/<slug>` — the skill body, followed by a manifest of the references it declares (names and addresses, not bodies).
- `hyprpilot://references/<slug>/<name>` — **one** reference, addressed by name.
- `hyprpilot://references/<slug>` — every reference that skill declares, bundled. Both reference forms are a parallel top-level scheme, not a `/references` segment nested under the slug — the nested form broke client URI autocomplete.

::: warning Reference URIs are templates, never enumerated

`resources/list` returns the catalogue index and one entry per skill — and nothing else. Neither reference form appears in it; both are advertised as **resource templates**, which cost one entry each regardless of how many skills or references exist.

That is a context-budget decision, measured against a 124-skill catalogue. Listing one entry per skill costs ~128 resources and ~105 KB. Adding a bundle entry per skill took it to 231 and ~170 KB, of which 48% was `_meta` — and each bundle entry's `_meta` was its own skill's block repeated verbatim, paying twice for one skill's metadata. Enumerating all 479 individual references on top would reach **~607 entries and ~500 KB — over 120k tokens spent before a single skill is read**.

Use `list_skill_references` to ask what a skill cites; it answers the same question for a fraction of the cost, and it is a call you choose to make rather than a cost every client pays on connect.

:::

And as tools:

| Tool                    | Purpose                                                              |
| ----------------------- | -------------------------------------------------------------------- |
| `list_skills`           | Enumerate discovered skills with their metadata and reference count. |
| `read_skill`            | Fetch a skill body by slug, plus its reference manifest.             |
| `list_skill_references` | Reference metadata without bodies, for one skill or every skill.     |
| `load_skill_references` | Fetch one, several, or all of a skill's reference bodies.            |
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

**Each one is addressed by name — the file name without its extension.** `../references/output-diff.md` becomes `output-diff`, which is the same word the skill body uses when it cites the convention ("present it per `output-diff`"), so the address and the prose agree. A reference may override that with a `name:` in its own frontmatter.

### Bodies are opt-in

`read_skill` returns the skill body plus a **manifest** — every declared reference with its name, address, size, and when it last changed — but not their bodies:

```jsonc
{
  "uri": "hyprpilot://skills/git-commit",
  "body": "…",
  "references": [
    {
      "name": "output-diff",
      "uri": "hyprpilot://references/git-commit/output-diff",
      "path": "/home/you/.config/hyprpilot/skills/references/output-diff.md",
      "size": 2481,
      "modified": "2026-08-04T09:12:33Z",
      "created": "2026-05-02T11:04:07Z"
    }
  ]
}
```

Fetch what the body actually directs you to:

```jsonc
// one, several
load_skill_references { "slug": "git-commit", "references": ["output-diff"] }
// all of them
load_skill_references { "slug": "git-commit" }
// body plus everything, in one call
read_skill { "slug": "git-commit", "bundle": true }
```

Omitting `references` fetches every reference; an **explicitly empty array fetches none** — an empty list must never decay into its opposite. An unknown name is an error listing what does exist, rather than a partial bundle the caller would mistake for a complete one.

The reason for opt-in is duplication, not size in the abstract: references are shared, so the same file is declared by many skills. Reading two related skills used to re-send conventions already in context. Loading one skill's body plus every reference costs several times what the body alone costs, and most steps need one reference, not all of them.

Because the manifest always rides along — including as a text footer on the resource path, for clients that never surface `_meta` — declining a body is never a silent gap. The reader can always see what exists and what it has not loaded.

### `path` is identity, and identity is what stops double-loading

A name is an **address**, not an identity. `git-commit/output-diff` and `git-push/output-diff` are the same file under two addresses; two skills' own `./references/output-diff.md` share a name and are different files. Only the resolved `path` separates those, so it is the field to compare when deciding whether you already hold a body. It is canonicalized, which collapses `..` so two spellings of one file compare equal.

`list_skill_references` exists for exactly that question — metadata for one skill, or for **every** skill when the `slug` is omitted:

```jsonc
list_skill_references { "slug": "git-commit" }   // one skill
list_skill_references {}                          // the whole catalogue
```

Its text projection groups citations by path and leads with the files cited by more than one skill, so the overlap is visible without diffing anything:

```txt
SHARED - 60 file(s) cited by more than one skill. Each group below is ONE
file: fetching a second citation re-sends what you already hold.
  /home/you/.config/hyprpilot/skills/references/output-diff.md
    git-commit/output-diff, git-push/output-diff, gitlab-mr-create/output-diff, …
```

Scanning the whole catalogue is much larger than one skill — worth doing once to learn what is shared, not on every turn. `list_skills` deliberately does not resolve references at all; it reports a `referenceCount` and is served purely from cache, so enumerating skills never touches the filesystem.

Only the **resolved** path is published. The declared spelling (`../references/output-diff.md`) is meaningless outside its bundle directory and never reaches the wire.

### Names, collisions, and missing files

- **Collision:** if two declared references resolve to the same name, the first wins. The loser keeps its manifest row marked `shadowed: true` with no address of its own, is still served by the full bundle, and is warned in the log. It is never silently dropped.
- **Missing file:** a reference that is declared but cannot be read appears in the bundle as a `status: not-found` block **in its declared position**, so the gap is visible where it belongs. Addressing it individually by URI errors instead, since a resource read has no in-band marker convention.
- **Reference frontmatter:** a reference may carry its own YAML frontmatter, parsed exactly as a skill's is. It is served with the fence stripped and its keys projected into the manifest entry's `metadata` — nothing is invented into it, because hyprpilot enforces no invocation gate and a stamped `disableModelInvocation` would imply a restriction that does not exist.

A fetched reference carries its **full** metadata: the bundle header is built from the same manifest row the listing advertises, so the two cannot disagree, and reading one as a resource returns that row under its own `io.hyprpilot/reference` key rather than its declaring skill's block.

```txt
---
reference:
  created: 2026-08-10T10:32:30Z
  modified: 2026-08-10T12:08:46Z
  name: output-diff
  path: /home/you/.config/hyprpilot/skills/references/output-diff.md
  size: 2011
  uri: hyprpilot://references/git-commit/output-diff
---
# Output Diff
…
```

Full detail is affordable there and not in a listing: it is emitted once per reference you deliberately asked for, whereas `resources/list` pays for the whole catalogue.

### Timestamps

Skills and references both carry `size`, `modified`, and `created` as RFC 3339 UTC strings, so an agent can tell a convention it read last week from one that changed an hour ago. `created` is the filesystem birth time and is **omitted** where the platform or filesystem does not record one, rather than being back-filled from `modified` — that would answer a different question than the key names. Access time is deliberately absent: it records reads rather than writes, and lazily on the `relatime` mounts that are the Linux default.

Skills are discovered by directory scan — the same discovery the launcher uses — so editing a skill and calling `reload` refreshes the catalogue without restarting the agent session. Because each skill is exposed as a resource, `reload` also emits a **resource list-changed notification** so a connected client re-fetches the skill list instead of trusting a stale one. (The tool list is static, so no tool-list-changed fires.)

That is the whole skills surface. The other two in-tree servers are separate processes with separate catalogue entries: `hyprpilot mcp serve` carries `open` (see [the `mcp` block](../config/mcp#mcp-serve)), and `hyprpilot mcp harness` carries `list_profiles` / `spawn` / `session_*` for launching and driving other hyprpilot sessions — off by default, documented in [Agent Harness](./harness).

Every tool result carries **both** a human-readable text block and the structured JSON payload. Clients that render only structured content (Claude Code) read the JSON; clients that render only text (opencode) get a legible summary — e.g. `list_skills` returns a one-line-per-skill catalogue as text alongside the full structured list, and `read_skill` returns the skill body as text alongside the structured `{ uri, body, metadata }`. A structured-only result would otherwise render as "Unknown" in text-only clients.

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
