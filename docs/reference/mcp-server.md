---
title: MCP server
order: 2
---

# The in-tree MCP server

`hyprpilot mcp serve` is a small [rmcp](https://github.com/modelcontextprotocol/rust-sdk)
stdio server whose one job today is to expose your **skills** catalogue
to the agent over MCP. Hyprpilot auto-injects it into the vendor's MCP
config (as a server named `hyprpilot`) whenever `[mcp].enabled` is true
and the resolved skills catalogue is non-empty — the agent vendor spawns
it as a stdio child and owns its lifetime. You never launch it by hand.

See [Configuration → MCP & skills](../configuration/mcp-and-skills) for
how to configure the skills roots and toggle auto-injection.

## How it's launched

The auto-injected entry runs the current binary with:

```sh
hyprpilot mcp serve --skill-dir '{"dir":"<root>","ignore":[…]}' …
```

One `--skill-dir` argument is passed per configured skills root, each
carrying that root's own ignore-glob list as JSON. The server rescans
those directories on start and on the `reload` tool, so editing a
`SKILL.md` and calling `reload` refreshes the catalogue without
restarting the agent session.

## Resources

Skills are exposed as MCP resources:

- `hyprpilot://skills/<slug>` — the skill body.
- `hyprpilot://skills/<slug>/references` — the bundle's declared
  reference files, resolved relative to the skill directory.

## Tools

| Tool | Purpose |
| --- | --- |
| `list_skills` | Enumerate discovered skills with their metadata. |
| `read_skill` | Fetch a skill body by slug. |
| `load_skill_references` | Bundle the reference files a skill declares. |
| `reload` | Rescan the skill roots (picks up edits / new bundles). |
| `open` | Open a URL, file, or directory in the OS default handler. |

## Frontmatter passthrough

A `SKILL.md` is markdown with an optional YAML frontmatter block. The
loader keeps **every** frontmatter key losslessly, and the server passes
the whole map through to the agent on the MCP wire under the resource's
`_meta`, so a new frontmatter field reaches the agent with zero server
changes:

- `io.hyprpilot/frontmatter` — the entire frontmatter map, verbatim.
  Keys pass through unchanged (no camelCasing); nested maps, arrays,
  numbers, and booleans all convert. The consumer interprets — renaming
  is interpretation the server deliberately doesn't do.
- `io.hyprpilot/skill` — a curated / derived view (name, interaction,
  argument hint, references, path, bundle dir) for consumers that want
  the pre-derived shape without re-deriving it themselves.

Both keys are namespaced per the MCP spec's `_meta` reverse-DNS
convention. Frontmatter that isn't map-shaped, or a `SKILL.md` with no
frontmatter fence at all, is treated as an empty map — a malformed block
never fails the request.

Example — this `SKILL.md`:

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

…reaches the agent with every one of those keys (`name`,
`disable-model-invocation`, the nested `metadata` map) intact under
`io.hyprpilot/frontmatter`.
