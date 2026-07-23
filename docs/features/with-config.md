---
title: Ad-hoc Overlays (--with-config)
order: 40
---

# {{ $frontmatter.title }}

If you want to bend a profile for a single launch — a different MCP set, one extra prompt, a model swap driven by a script — use the repeatable `--with-config` flag. Each value is a partial profile overlay folded onto the resolved profile, **after** the root [`[[patches]]`](./patches).

<!-- more -->

## Input shapes

```sh
hyprpilot -p engineer --with-config ./overlay.toml
hyprpilot -p engineer --with-config '@{"model":"claude-opus-4-5"}'
some-generator | hyprpilot -p engineer --with-config -
```

Each value is one of three shapes:

- **a file path** — the extension (`.toml` / `.json` / `.yaml` / `.yml`) drives the format;
- **`@<inline body>`** — an inline literal in the current format;
- **`-`** — read from stdin, usable **at most once** per invocation.

The flag is repeatable; overlays fold in declaration order, later wins on field collision.

## `--with-config-format`

`--with-config-format toml|json|yaml` drives stdin, inline, and extension-less inputs. It defaults to `json` — the best fit for CLI piping and inline one-liners:

```sh
gh api …upstream-config… | jq '{mcps: [.]}' | hyprpilot -p engineer --with-config -
```

## Merge semantics

Overlays use the same strategic-merge engine as `[[patches]]` — object-field merge, keyed-array merge by `id`, primitive-array append + dedupe, and the `$patch: "replace"` / `$deleteFromPrimitiveList/<field>` directives. See [Patches → Merge semantics](./patches#merge-semantics).

## Where it sits in resolution

`--with-config` overlays are folded **after** the root `[[patches]]` — they are the most specific config layer, beaten only by the direct CLI flags (`--model`, `--mode`, `--cwd`, `--agent`) applied on top of the resolved profile.
