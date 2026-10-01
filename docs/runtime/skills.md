---
title: Skills & the hyprpilot MCP Server
order: 50
---

# {{ $frontmatter.title }}

Skills are `SKILL.md` bundles — reusable markdown instructions the agent can list and read, and whose roots the sidecar watches so edits announce themselves. They reach the agent **only** through hyprpilot's own in-tree MCP server, which the launcher auto-injects into the vendor's MCP config. The same server also serves **prompts** — including the launched profile's own system prompt — so an edited prompt can be re-read mid-session.

<!-- more -->

The catalogue is served over stdio to the vendor that spawned it. To share one catalogue with other clients instead, see [Serving MCP over HTTP](./mcp-http).

## Skill bundles

The skills catalogue is configured under the [`mcp` block](../config/mcp#the-mcp-block). Every directory under a configured root that holds a `SKILL.md` is a skill, at any depth, following the [Agent Skills specification](https://agentskills.io/specification):

```txt
~/.config/hyprpilot/skills/
├── git-commit/
│   └── SKILL.md
├── linear-issue/
│   ├── SKILL.md
│   ├── references/
│   │   └── triage.md
│   └── scripts/
│       └── fetch.py
└── acme/                     # an organizational prefix, not a skill
    └── billing/
        └── refunds/
            └── SKILL.md      # the skill `acme/billing/refunds`
```

A skill is identified by its **path** under the root — `git-commit`, or `acme/billing/refunds` for a nested one. That path is its slug on every tool and the `<skill-path>` of its `skill://` URIs. A skill may also nest inside another; the enclosing skill then ships the nested one's files as its own, as [SEP-2640](https://github.com/modelcontextprotocol/modelcontextprotocol/blob/main/seps/2640-skills-extension.md) requires.

A `SKILL.md` must carry YAML frontmatter with a `name` equal to its directory and a non-empty `description`. A skill missing either, or whose `name` differs from its directory, is skipped with a warning: the frontmatter is served verbatim, and a SEP-2640 host refuses an entry like that. The name follows the Agent Skills rule — 1-64 of `[a-z0-9-]`, no leading, trailing or doubled hyphen. Organizational prefixes are lowercase too, because the first one becomes a URI authority.

Hidden entries and anything a `.gitignore` (or `.ignore`) excludes are never discovered or served, so a script's `.venv` stays out. Symlinks are not followed. A bundle past SEP-2640's limits — 512 files or 16 MiB — is not served, and a warning says so.

Per-root `include` globs keep only matching skill paths and per-root `ignore` globs skip them; a path matching both is skipped. Globs match the whole path and `*` crosses `/`, so `acme/*` matches every skill under `acme`. Ignoring a skill does not ignore skills nested inside it — each is matched on its own path. On a path collision across roots, the first root wins. Missing roots warn and are skipped.

The compiled defaults seed the XDG skills root `~/.config/hyprpilot/skills` (via a root [`patches`](../config/patches) entry), and the built-in `mcp` defaults (`enabled: true`, `autoAcceptTools: ['*']`) fill in the rest — so skills work out of the box once you drop a `SKILL.md` in. A profile's own `mcp` block wholesale-replaces the global one — point a profile at a different skills root, or disable the server entirely.

## Prompts

The server also serves prompts, from two sources:

- **The profile's own `system_prompt` files**, on by default (`mcp.skills.system_prompts`). The launcher still bakes them into the vendor at launch — that cannot change mid-session — but served as a prompt, an edited system prompt can be re-invoked without a relaunch. Every entry passes, including `inject: false` ones.
- **`[[mcp.skills.prompts]]` directories.** Every `*.md` directly inside one is a prompt. The entry carries the same `include` / `ignore` / `watch` keys a skill root does, matched against the prompt name.

A prompt is named by its frontmatter `name`, else its file stem (`AGENTS.md` is `AGENTS`), and must fit `[A-Za-z0-9_.-]{1,128}`. Its body is served with any frontmatter fence stripped; `title` and `description` in that frontmatter are listed with it. The profile's files come first, so a directory can never shadow the system prompt; after that, the first prompt to claim a name keeps it.

Prompts reach a client two ways, because clients disagree about prompts:

| Client      | MCP prompts (`prompts/list`, `prompts/get`)                     | `hyprpilot://prompts/<name>` resources |
| ----------- | --------------------------------------------------------------- | -------------------------------------- |
| Claude Code | `/mcp__hyprpilot-skills__<name>` slash commands, refreshed live | `@`-mentions and resource tools        |
| opencode    | slash commands, refreshed on reconnect                          | `@`-mentions and a resource tool       |
| Codex       | ignored                                                         | resource tools                         |
| Hermes      | model-callable `get_prompt` tools                               | resource tools                         |

Prompt files are watched: a prompt directory directly, and a single prompt file through its parent directory, so an editor's atomic save — which replaces the file — is still seen.

## Auto-injection

When `mcp.enabled` is `true`, `mcp.skills.enabled` is `true` (the default), **and** there is at least one skill or one loadable prompt to serve, hyprpilot prepends a stdio MCP server named **`hyprpilot-skills`** to the catalogue it hands the vendor. That entry launches `hyprpilot mcp skills` as a child of the agent — the vendor owns its lifetime; you never run it by hand.

- The reserved name replaces any same-named server you configured. Rename it with `mcp.skills.name`.
- Auto-inject is independent of `mcps` — `mcps: []` does not suppress it. Set `mcp.skills.enabled: false` (this server only), `mcp.enabled: false` (every in-tree server), or leave both the catalogue and the prompts empty to turn it off.
- This server is also gated on **content**: nothing to serve means nothing is injected. A profile with a `system_prompt` therefore gets the server even with no skills, because that prompt is served.
- `autoAcceptTools` / `autoRejectTools` default the approval policy for the injected server; the default `['*']` accept makes skill calls frictionless.

The injected entry runs the current binary with one `--skill-dir` per skill root, one `--prompt-dir` per prompt directory and one `--prompt-file` per `system_prompt` file — see [the `mcp skills` reference](#hyprpilot-mcp-skills) below for the exact shape.

## What the server exposes

`hyprpilot mcp skills` is a small [rmcp](https://github.com/modelcontextprotocol/rust-sdk) stdio server.

### SEP-2640 skills

The server implements the [Skills Extension](https://github.com/modelcontextprotocol/modelcontextprotocol/blob/main/seps/2640-skills-extension.md) and declares it as `capabilities.extensions["io.modelcontextprotocol/skills"] = { "directoryRead": true }`:

- Every file of a bundle is a resource at `skill://<skill-path>/<file-path>`, served **raw** — `SKILL.md` frontmatter included — because a host verifies what it reads against a digest and re-parses the frontmatter against the listing.
- `skills/list` returns every skill as `{ uri, frontmatter, resources }`: the `SKILL.md` URI, the frontmatter verbatim as JSON, and every file with its `sha256:` digest and byte size. The listing is a single page and carries `ttlMs` and `cacheScope`.
- `skills/get { uri }` returns one entry; a URI that is not a skill's `SKILL.md` is `-32602`.
- `resources/directory/read { uri }` lists a directory's direct children: files with their metadata, subdirectories as `inode/directory`. Every directory counts — a skill's root, any subdirectory, and an organizational prefix such as `skill://acme`. A file or an unknown path is `-32602`.

File bytes are read once per rescan and served from memory, so what a read returns always matches the digest the listing promised. An unchanged file (same size and modification time) is carried over rather than read and hashed again.

::: info Which clients use it

As of October 2026 no shipping client turns this on by default. Claude Code 2.1.286 carries a client behind a disabled feature flag, and it caps a server at 100 skills; opencode, Codex and Hermes do not implement it. The tools and resources below are what reaches the model everywhere today — the SEP surface is there for when clients enable it.

:::

### Resources

- `hyprpilot://skills` — the **catalogue index**: every skill with its description, as one markdown document, led by a header explaining how to load them. Attach it (`@`-mention it, or whatever your client calls that) and it costs **no** tool call — the client injects it directly.
- `skill://<skill-path>/SKILL.md` — one per skill, listed. Any other bundle file is readable by its `skill://` URI but not listed.
- `hyprpilot://prompts/<name>` — one per prompt, listed.
- `file://<path>` — a shared reference some skill declares (see [References](#references)). Readable, never listed.

::: warning Only skills and prompts are listed, and that is a context-budget decision

Measured against a real 127-skill catalogue: listing one entry per skill costs 128 resources and ~105 KB. Adding one more entry per skill took it to 231 and ~170 KB, and enumerating all 479 individual references would reach **~607 entries and ~500 KB, over 120k tokens spent before a single skill is read**. Bundle files and references are reachable by URI, by `skills/list`, and by directory read instead.

:::

### Tools

| Tool                    | Purpose                                                                                                          |
| ----------------------- | ---------------------------------------------------------------------------------------------------------------- |
| `list_skills`           | Enumerate discovered skills with their metadata, reference count and file count.                                 |
| `read_skill`            | Fetch a skill's body (frontmatter stripped) by slug, plus manifests of its references and of the files it ships. |
| `list_skill_references` | One skill's reference metadata, without bodies.                                                                  |
| `read_skill_references` | Fetch reference bodies by path.                                                                                  |
| `read_skill_files`      | Fetch files a skill ships — scripts, templates, its own references — by `skill://` URI.                          |
| `reload`                | Force a rescan. The roots are watched, so this is the fallback for a root reported degraded or off.              |

`read_skill_files` is what makes a bundle's own files reachable for a client that only calls tools, or one talking to the server over HTTP that cannot read the bundle's directory. A file that is not UTF-8 is described rather than inlined; read it with `resources/read`, which serves it as a blob.

### Watching

Every configured root is watched recursively, and this is on by default. A change under one is coalesced over a 500 ms quiet window, rescanned, and announced — so an edit reaches connected clients without anyone calling a tool.

Two things make watching affordable rather than noisy. Changes under a hidden entry (a `.venv`, an editor's swap file) and plain file opens never wake the sidecar, and the **diff** decides what goes on the wire: a rescan that moved nothing announces nothing.

A root can lose coverage, and the sidecar keeps serving when it does:

| Situation                                                                                                             | State      |
| --------------------------------------------------------------------------------------------------------------------- | ---------- |
| Normal                                                                                                                | `watching` |
| Root does not exist, the inotify watch limit was reached, the watcher thread exited, or the backend reported an error | `degraded` |
| `watch = false` on that root                                                                                          | `off`      |

`list_skills` reports this as a `watch` object (`{ active, roots }`), and its text summary names any uncovered root. `active` is true only when there is at least one root and **every** one of them is covered. When it is not, `reload` is the way to refresh.

An error the backend attributes to a path degrades only the roots that path falls under; one it cannot attribute degrades all of them, which is the only case where blaming a root that may be fine is honest.

Two cases a watch cannot cover, both of which are what `reload` is for:

- **A root on a filesystem that cannot deliver events** — NFS, SSHFS, most FUSE mounts accept the watch and then never fire. There is no error to detect, so set `watch = false` on that root and use `reload`.
- **A reference file outside every configured root.** The watch covers each root recursively; a skill citing a path above its root still serves that file fresh on every fetch, but a change to it is not announced.

### What a rescan tells connected clients

Results carry a `ttlMs` of 24 hours — longer than a sidecar lives — so a client caches until told otherwise. Every rescan — the watcher's, or a `reload` — earns that by **diffing** and firing only what actually changed:

| What you changed                  | What fires                                                                                                                           |
| --------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------ |
| A skill's `SKILL.md`              | `resources/updated` for its `SKILL.md` URI and for the catalogue index, plus `resources/list_changed`                                |
| A file a skill ships              | `resources/updated` for that file's URI, the skill's `SKILL.md` URI (its `resources` set changed) and the index, plus `list_changed` |
| Added or removed a skill          | `resources/list_changed`, plus `resources/updated` for the index                                                                     |
| A reference file a skill declares | `resources/updated` for the reference's `file://` URI, plus `resources/list_changed` — but **not** the index                         |
| A prompt                          | `prompts/list_changed`, `resources/updated` for its `hyprpilot://prompts/<name>` URI, plus `resources/list_changed`                  |
| Nothing                           | nothing — a rescan that moved nothing never invalidates a client's cache                                                             |

A file a skill shares with a skill nested inside it is one path, so it is announced once. MCP has no per-prompt update, so an edited prompt body is a prompt **list** change — the only way a client learns to re-fetch it.

The `reload` result reports the same thing (`{ reloaded, prompts, membershipChanged, updated, filesChanged, referencesChanged, promptsChanged, watch }`), so you can see what a rescan actually moved.

A client on `2026-07-28` opts in with `subscriptions/listen` (`resourcesListChanged`, `promptsListChanged` and/or `resourceSubscriptions`), and its notifications then ride that stream, tagged with the subscription id. A client with no stream — anything on an older revision — receives them as plain unsolicited notifications.

`resources/list_changed` fires on **any** change, not only on membership, precisely so a client that cannot subscribe still has a signal it can act on.

Reference **bodies** stay uncached — they resolve from disk on every fetch, so `modified` is always live. What the cache holds is each declared file's size and modification time, read once per rescan (one `metadata()` per unique file, however many skills cite it). The comparison uses the raw modification time, not the seconds-truncated string it serves.

A rescan refreshes the **sidecar**, not anything already in an agent's context — a skill body read earlier this session stays as it was until re-read. The notification is what tells a client to re-read; acting on it is the client's own behaviour.

## References

A skill declares SHARED references in frontmatter, as paths relative to the skill's own directory:

```markdown
---
name: git-commit
description: Stage and commit changes
references:
  - ../references/commit-style.md
  - ../references/output-diff.md
---
```

`references:` is hyprpilot's own key, not part of the Agent Skills specification: it names files outside the bundle, which many skills share. A file a skill ships inside its own directory needs no declaration — it is a bundle file, served at its `skill://` URI.

### The path is the address, and the identity

`read_skill` returns the skill body plus a **manifest** — every declared reference, with the canonical path that fetches it — but not their bodies:

```jsonc
{
  "uri": "skill://git-commit/SKILL.md",
  "body": "…",
  "references": [
    {
      "path": "/home/you/.config/hyprpilot/skills/references/output-diff.md",
      "uri": "file:///home/you/.config/hyprpilot/skills/references/output-diff.md",
      "name": "output-diff",
      "size": 2481,
      "modified": "2026-08-04T09:12:33Z",
      "created": "2026-05-02T11:04:07Z"
    }
  ],
  "files": [{ "uri": "skill://git-commit/scripts/check.py", "size": 812, "mimeType": "text/x-python" }]
}
```

Pass those paths back to fetch bodies:

```jsonc
read_skill_references { "references": ["/…/references/output-diff.md"] }
// body plus everything, in one call
read_skill { "slug": "git-commit", "bundle": true }
```

Addressing by path rather than by skill-and-name buys three things:

- **De-duplication.** The same shared file is cited by many skills under different names. Two citations resolve to one path, so a path you already loaded needs no second fetch — and the server serves a repeated path once.
- **One call across skills.** A path names a file, not a skill, so a single call fetches references belonging to as many skills as you like.
- **No collision rules.** Paths are unique by construction, so two references sharing a label inside one skill are both fully addressable.

A reference is also a resource: its `file://` URI (in the manifest row as `uri`) reads through `resources/read`. `skill://` cannot name it, because that scheme addresses files inside one skill.

Only paths that some skill actually declares are served — a caller-supplied path is checked against that set, never joined onto anything, so the surface reaches exactly the files the skills already reference. Anything else is an error rather than a partial result.

The **declared** spelling (`../references/output-diff.md`) never reaches the tool output: it is meaningless outside its bundle directory. Paths are canonicalized, so `..` collapses and two spellings of one file compare equal. The raw `SKILL.md` resource does still carry it, because that resource is the file verbatim.

`list_skill_references { slug }` returns the same manifest without the skill body, for checking what a skill cites before spending tokens on it.

Because the manifests always ride along — including as text footers on `read_skill`, for clients that never surface structured content — declining a body is never a silent gap.

### Missing files and reference frontmatter

- **Missing file:** a reference that is declared but cannot be read appears in the manifest and in any bundle as a `status: not-found` marker **in its declared position**, so the gap is visible where it belongs. It has no path, so it cannot be fetched.
- **Reference frontmatter:** a reference may carry its own YAML frontmatter, parsed exactly as a skill's is. It is served with the fence stripped and its keys projected into the manifest entry's `metadata` — nothing is invented into it. A `name:` there overrides the display label.

A fetched reference carries its **full** metadata: the bundle header is built from the same manifest row the listing advertises, so the two cannot disagree.

```txt
---
reference:
  path: /home/you/.config/hyprpilot/skills/references/output-diff.md
  uri: file:///home/you/.config/hyprpilot/skills/references/output-diff.md
  name: output-diff
  size: 2011
  modified: 2026-08-10T12:08:46Z
  created: 2026-08-10T10:32:30Z
---
# Output Diff
…
```

### Timestamps

Skills and references both carry `size`, `modified`, and `created` as RFC 3339 UTC strings, so an agent can tell a convention it read last week from one that changed an hour ago. `created` is the filesystem birth time and is **omitted** where the platform or filesystem does not record one, rather than being back-filled from `modified`. Access time is deliberately absent: it records reads rather than writes.

## Frontmatter passthrough

The loader keeps **every** frontmatter key losslessly. `skills/list` carries the map verbatim, as SEP-2640 requires. Everywhere else, metadata is carried in **one** block — never duplicated across surfaces:

- **Spec `Resource` fields** are canonical: `uri`, `name` (the frontmatter name, which is the skill path's last segment), `title`, `description`, `mimeType`, `size`.
- **`io.hyprpilot/skill`** (resource `_meta`) / **`metadata`** (tool output) — the same single block: the entire frontmatter map **verbatim** (keys pass through unchanged — no camelCasing; nested maps, arrays, numbers, and booleans all convert), **minus** the keys another field already carries, **plus** the runtime-derived `path`, `bundleDir`, `size`, `modified`, and `created`.

Three frontmatter keys are dropped from the block as duplicates. `title` and `description` equal the canonical `Resource.title` / `Resource.description`. `references` is superseded by the resolved [reference manifest](#references), which addresses each one by its canonical path.

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

…reaches the agent with `title` / `description` on the spec `Resource` fields, and every other key (`name`, `disable-model-invocation`, the nested `metadata` map) plus the runtime `path` / `bundleDir` intact under the single `io.hyprpilot/skill` block.

:::

## `hyprpilot mcp skills`

The subcommand that runs the server over stdio. **You don't run this by hand** — the agent vendor spawns it as a child via the auto-injected entry.

```sh
hyprpilot mcp skills --skill-dir '{"dir":"/abs/path","ignore":[],"watch":true}' --prompt-file ~/AGENTS.md
```

| Flag                   | Purpose                                                                              |
| ---------------------- | ------------------------------------------------------------------------------------ |
| `--skill-dir <json>`   | JSON-encoded skill root entry. Repeatable — roots are searched in declaration order. |
| `--prompt-dir <json>`  | JSON-encoded prompt directory, the same shape. Repeatable.                           |
| `--prompt-file <path>` | One prompt file. Repeatable; earlier files claim a name first.                       |

Each `--skill-dir` / `--prompt-dir` value is one self-contained JSON object:

```json
{ "dir": "/abs/path", "include": ["glob1"], "ignore": ["glob2"], "watch": true }
```

The launcher passes one per resolved root, each carrying that root's own include and ignore glob lists and watch flag, so the sidecar rebuilds exactly what the launcher resolved — first path wins on collision, per-root filters applied independently. An absent or empty `include` means no allow-list, never "allow nothing". `watch` defaults to `true`, so a hand-written catalogue entry that omits it still gets a watched root.

The [global flags](./launch#global-flags) apply here too; the server owns stdin/stdout for the MCP protocol, so logs go to stderr as everywhere else.
