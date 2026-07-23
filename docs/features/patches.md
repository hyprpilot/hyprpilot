---
title: Patches
order: 30
---

# {{ $frontmatter.title }}

If you want the same knob on several profiles — a shared system prompt, a team MCP file — don't repeat it per profile: put it in a root-level `[[patches]]` entry. A patch is a partial profile shape that merges onto whichever profile gets picked at resolve time.

<!-- more -->

This is the single mechanism for **profile-shared knobs** — there is deliberately no root-level `system_prompt` / `mcps` / `mcp` field.

## Writing a patch

```toml
# Unscoped — applies to every profile.
[[patches]]
[[patches.system_prompt]]
file = "~/.config/hyprpilot/prompts/base.md"

# Scoped — only profiles whose id matches the glob.
[[patches]]
"$match" = { profile = "work/*" }

[[patches.mcps]]
file = "~/.config/hyprpilot/mcps/work.json"
```

Anything you can write under `[[profiles]]` you can write in a patch — the patch body is the same partial-profile shape.

## `$match`

An optional `$match` sibling filters where a patch applies and is stripped before the merge so it never lands on the profile shape:

- `$match.profile = "<glob>"` — apply only to profiles whose id matches the glob. Globs cross `/`, so `work/*` matches `work/claude/opus`.
- Unset `$match` (or unset `profile`) — apply to every profile.

Patches fold left-to-right in declaration order; a later patch wins on field collision.

## Merge semantics

Patches fold with a strategic-merge engine — the same one [`--with-config`](./with-config) uses:

- **Object fields** merge recursively; a scalar leaf on the patch overwrites the base.
- **Keyed arrays** (entries carrying an `id`) merge by `id` — a patch entry with a matching `id` merges onto the base entry, new ids append.
- **Primitive arrays** append and de-duplicate.
- **`$patch: "replace"`** on an object or array forces a wholesale replace instead of a merge — use it on the profile side to wipe a patch-provided list:

  ```toml
  [[profiles]]
  id = "clean"
  agent = "claude-code"

  [profiles.env]
  "$patch" = "replace" # ignore any env a patch would overlay
  ```

- **`$deleteFromPrimitiveList/<field>`** removes specific entries from a primitive array.

## The default patch

The compiled defaults seed one unscoped patch that enables the in-tree `hyprpilot` MCP server with auto-accept-everything and the XDG skills directory — this is why [skills](./skills) work out of the box:

```toml
[[patches]]
[patches.mcp]
enabled = true
autoAcceptTools = ["*"]
autoRejectTools = []

[[patches.mcp.skills]]
dir = "~/.config/hyprpilot/skills"
```

Add your own `[[patches]]` entries (later wins) or override the `mcp` field per-profile to change it.

## Where patches sit in resolution

Every launch resolves the effective profile through one path:

1. Pick the base profile — `--profile <id>` first, then `[profile] default`. Errors when neither names a real `[[profiles]]` entry.
2. Fold each root `[[patches]]` entry (filtered by its `$match.profile` glob) in declaration order.
3. Fold each [`--with-config`](./with-config) overlay in declaration order.
4. Deserialize the merged result back into a profile and re-validate.

`--agent <id>` wins over whatever agent the patched profile names.
