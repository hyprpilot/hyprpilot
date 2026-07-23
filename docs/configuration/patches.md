---
title: '[[patches]]'
order: 40
---

# {{ $frontmatter.title }}

Root-level partial-profile overlays, folded onto whichever profile a launch picks. Narrative: [Features → Patches](../features/patches).

<!-- more -->

```toml
[[patches]]
"$match" = { profile = "work/*" }

[[patches.mcps]]
file = "~/.config/hyprpilot/mcps/work.json"
```

## Shape

Each `[[patches]]` entry is a partial `[[profiles]]` shape — any profile field is valid — plus one optional control sibling:

| Field    | Type              | Default | What it does                                                            |
| -------- | ----------------- | ------- | ----------------------------------------------------------------------- |
| `$match` | object (optional) | unset   | Filters which profiles the patch applies to; stripped before the merge. |
| _(rest)_ | partial profile   | —       | Fields folded onto the picked profile with the strategic-merge engine.  |

### `$match`

| Field     | Type            | Default | What it does                                                                                  |
| --------- | --------------- | ------- | --------------------------------------------------------------------------------------------- |
| `profile` | glob (optional) | unset   | Profile-id glob (crosses `/`, so `work/*` matches `work/claude/opus`). Unset = every profile. |

## Merge directives

Both `[[patches]]` and `--with-config` fold through the same strategic-merge engine:

| Directive                          | Where            | Effect                                          |
| ---------------------------------- | ---------------- | ----------------------------------------------- |
| _(none)_                           | objects          | Recursive field merge; scalar leaves overwrite. |
| _(none)_                           | keyed arrays     | Merge by `id`; new ids append.                  |
| _(none)_                           | primitive arrays | Append + de-duplicate.                          |
| `"$patch" = "replace"`             | object / array   | Wholesale replace instead of merge.             |
| `$deleteFromPrimitiveList/<field>` | primitive arrays | Remove the listed entries from the base array.  |

## Ordering

Patches fold left-to-right in declaration order, after the profile is picked and before `--with-config` overlays; a later patch wins on field collision.

## The seeded patch

The compiled defaults ship one unscoped patch enabling the in-tree `hyprpilot` MCP server (`enabled = true`, `autoAcceptTools = ["*"]`) with the XDG skills root — see it verbatim in the [reference overview](./#sources).
