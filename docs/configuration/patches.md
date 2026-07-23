---
title: Patches & overlays
order: 5
---

# Patches & overlays

Profiles are the base unit of config, but two mechanisms layer partial
overrides on top of whichever profile gets picked:

- **`[[patches]]`** — profile overlays stored in your config file,
  applied every launch.
- **`--with-config`** — profile overlays supplied per invocation on the
  command line.

Both are partial `ProfileConfig` shapes folded onto the resolved profile
with the same strategic-merge engine, so anything you can write under
`[[profiles]]` you can write in a patch.

## Root-level `[[patches]]`

A `[[patches]]` entry is a partial profile that merges onto whichever
profile is picked at resolve time. This is the single mechanism for
**profile-shared knobs** — there is no root-level `system_prompt` /
`mcps` / `mcp` field; put shared settings in a (possibly scoped) patch
instead of repeating them per profile.

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

### `$match`

An optional `$match` sibling filters where a patch applies and is
stripped before the merge so it never lands on the profile shape:

- `$match.profile = "<glob>"` — apply only to profiles whose id matches
  the glob. Globs cross `/`, so `work/*` matches `work/claude/opus`.
- Unset `$match` (or unset `profile`) — apply to every profile.

Patches fold left-to-right in declaration order; a later patch wins on
field collision.

### The default patch

The compiled defaults seed one unscoped patch that enables the in-tree
`hyprpilot` MCP server with auto-accept-everything and the XDG skills
directory — this is why skills work out of the box. Add your own
`[[patches]]` entries (later wins) or override the `mcp` field
per-profile to change it.

## `--with-config`

`--with-config` is a repeatable launch flag that folds a profile overlay
**after** the root `[[patches]]`, for per-invocation overrides:

```sh
hyprpilot -p engineer --with-config ./overlay.toml
hyprpilot -p engineer --with-config '@{"model":"claude-opus-4-5"}'
some-generator | hyprpilot -p engineer --with-config -
```

Each value is one of three input shapes:

- **a file path** — the extension (`.toml` / `.json` / `.yaml` / `.yml`)
  drives the format;
- **`@<inline body>`** — an inline literal in the current format;
- **`-`** — read from stdin, usable **at most once** per invocation.

`--with-config-format toml|json|yaml` (default `json`) drives stdin,
inline, and extension-less inputs. The flag is repeatable (except `-`);
overlays fold in declaration order.

## Merge semantics

Both `[[patches]]` and `--with-config` use the same strategic-merge
engine:

- **Object fields** merge recursively; a scalar leaf on the patch
  overwrites the base.
- **Keyed arrays** (`[[profiles]]`-style `id` lists, e.g. `mcps`) merge
  by `id`.
- **Primitive arrays** append and de-duplicate.
- **`$patch: "replace"`** on an object or array forces a wholesale
  replace instead of a merge — use it to wipe a patch-provided list:

  ```toml
  [[profiles]]
  id = "clean"
  agent = "claude-code"
  [profiles.clean.env]
  "$patch" = "replace"        # ignore any env a patch would overlay
  ```

- **`$deleteFromPrimitiveList/<field>`** removes specific entries from a
  primitive array.

## Resolution order

Every launch resolves the effective profile through one path:

1. Pick the base profile — `--profile <id>` first, then
   `[profile] default`. Errors when neither names a real `[[profiles]]`
   entry.
2. Fold each root `[[patches]]` entry (filtered by its `$match.profile`
   glob) in declaration order.
3. Fold each `--with-config` overlay in declaration order.
4. Deserialize the merged result back into a profile and re-validate.

`--agent <id>` wins over whatever agent the patched profile names.
