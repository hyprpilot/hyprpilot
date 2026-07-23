---
title: Patches
order: 40
---

# {{ $frontmatter.title }}

If you want the same knob on several profiles — a shared system prompt, a team MCP file — don't repeat it per profile: put it in a root-level `patches` entry. A patch is a partial profile shape that merges onto whichever profile gets picked at resolve time.

<!-- more -->

This is the single mechanism for **profile-shared knobs** — there is deliberately no root-level `system_prompt` / `mcps` / `mcp` field.

## Writing a patch

```yaml
patches:
  # Unscoped — applies to every profile.
  - system_prompt:
      - file: ~/.config/hyprpilot/prompts/base.md

  # Scoped — only profiles whose id matches the glob.
  - $match:
      profile: work/*
    mcps:
      - file: ~/.config/hyprpilot/mcps/work.json
```

Anything you can write under a `profiles` entry you can write in a patch — the patch body is the same partial-profile shape.

## Shape

Each `patches` entry is a partial profile shape — any profile field is valid — plus one optional control sibling:

| Field    | Type              | Default | What it does                                                            |
| -------- | ----------------- | ------- | ----------------------------------------------------------------------- |
| `$match` | object (optional) | unset   | Filters which profiles the patch applies to; stripped before the merge. |
| _(rest)_ | partial profile   | —       | Fields folded onto the picked profile with the strategic-merge engine.  |

### `$match`

| Field     | Type            | Default | What it does                                                                                  |
| --------- | --------------- | ------- | --------------------------------------------------------------------------------------------- |
| `profile` | glob (optional) | unset   | Profile-id glob (crosses `/`, so `work/*` matches `work/claude/opus`). Unset = every profile. |

Patches fold left-to-right in declaration order; a later patch wins on field collision.

## Additive across layers

`patches` accumulates across [config layers](./layering) instead of overwriting: the compiled defaults' patches come first, then your global config's, then the named config-layer profile's — each layer **appends** to the list, and the whole accumulated list folds onto the picked profile in that order.

That means a config-layer profile can add a work-only MCP patch without wiping the seeded default patch or your global ones. It also means you cannot delete an earlier layer's patch by redeclaring the list — to neutralize an inherited patch, add a later patch that overrides the same fields, using the `$patch: replace` directive to wipe rather than merge:

```yaml
# In a later layer: undo an inherited patch's extra prompts for `scratch`.
patches:
  - $match:
      profile: scratch
    system_prompt:
      $patch: replace
```

## Merge semantics

Patches fold with a strategic-merge engine — the same one [`--with-config`](../runtime/with-config) uses:

| Directive                          | Where            | Effect                                          |
| ---------------------------------- | ---------------- | ----------------------------------------------- |
| _(none)_                           | objects          | Recursive field merge; scalar leaves overwrite. |
| _(none)_                           | keyed arrays     | Merge by `id`; new ids append.                  |
| _(none)_                           | primitive arrays | Append + de-duplicate.                          |
| `$patch: replace`                  | object / array   | Wholesale replace instead of merge.             |
| `$deleteFromPrimitiveList/<field>` | primitive arrays | Remove the listed entries from the base array.  |

`$patch: replace` also works on the profile side — a profile can shield a field from patch overlays:

```yaml
profiles:
  - id: clean
    agent: claude-code
    env:
      $patch: replace # ignore any env a patch would overlay
```

## The default patch

The compiled defaults seed one unscoped patch that enables the in-tree `hyprpilot` MCP server with auto-accept-everything and the XDG skills directory — this is why [skills](../runtime/skills) work out of the box:

```yaml
patches:
  - mcp:
      enabled: true
      autoAcceptTools:
        - '*'
      autoRejectTools: []
      skills:
        - dir: ~/.config/hyprpilot/skills
```

Because patches accumulate, your own `patches` entries land **after** this seed (later wins on field collision) — or override the `mcp` field per-profile.

## Where patches sit in resolution

Every launch resolves the effective profile through one path:

1. Pick the base profile — `--profile <id>` first, then `profile.default`. Errors when neither names a real `profiles` entry.
2. Fold each accumulated `patches` entry (filtered by its `$match.profile` glob) in declaration order.
3. Fold each [`--with-config`](../runtime/with-config) overlay in declaration order.
4. Deserialize the merged result back into a profile and re-validate.

`--agent <id>` wins over whatever agent the patched profile names.
