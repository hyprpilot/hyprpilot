---
title: Ad-hoc Overlays (--with-config)
order: 40
---

# {{ $frontmatter.title }}

If you want to bend a profile for a single launch — a different MCP set, one extra prompt, a model swap driven by a script — use the repeatable `--with-config` flag. Each value is a partial profile overlay folded onto the resolved profile, **after** the root [`patches`](../config/patches).

<!-- more -->

## Input shapes

```sh
hyprpilot engineer --with-config ./overlay.yaml
hyprpilot engineer --with-config '@{"model":"claude-opus-4-5"}'
some-generator | hyprpilot engineer --with-config -
```

Each value is one of three shapes:

- **a file path** — the extension (`.toml` / `.json` / `.yaml` / `.yml`) drives the format;
- **`@<inline body>`** — an inline literal in the current format;
- **`-`** — read from stdin, usable **at most once** per invocation.

The flag is repeatable; overlays fold in declaration order, later wins on field collision.

## `--with-config-format`

`--with-config-format toml|json|yaml` drives stdin, inline, and extension-less inputs. It defaults to `json` — the best fit for CLI piping and inline one-liners:

```sh
gh api …upstream-config… | jq '{mcps: [.]}' | hyprpilot engineer --with-config -
```

## Merge semantics

Overlays use the same strategic-merge engine as [`patches`](../config/patches) — object-field merge, keyed-array merge by `id`, primitive-array append + dedupe, and the `$patch: replace` / `$patch: delete` / `$deleteFromPrimitiveList/<field>` directives. See [Config → Patches → Merge semantics](../config/patches#merge-semantics).

## Where it sits in resolution

`--with-config` overlays are folded **after** the root `patches` — they are the most specific config layer. Because the profile owns its agent and model, `--with-config` is _the_ way to change either for one launch (there are no `--agent` / `--model` flags); the only knobs applied on top of the resolved profile afterwards are `--mode` and `--cwd`.
