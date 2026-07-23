---
title: Reference Overview
order: 1
prev: false
---

# {{ $frontmatter.title }}

This section is the field-by-field reference for every config section — key, type, default, and behavior. For the narrative versions ("why would I use this?"), start at [Features](../features/layering) instead.

<!-- more -->

## Sources

Config resolves in layers — compiled defaults → global config → named config-layer profile → `[[patches]]` / `--with-config` — with later layers overriding earlier ones per field. [Config Layering](../features/layering) covers discovery, extension priority, and the merge rules.

Defaults quoted in these tables come from the compiled `src/config/defaults.toml`, the single source of truth:

::: details The compiled defaults, verbatim

<<< @/../src/config/defaults.toml

:::

## Root sections

| Section          | Shape               | Purpose                                                                                     |
| ---------------- | ------------------- | ------------------------------------------------------------------------------------------- |
| `[[agents]]`     | list, keyed by `id` | Vendor CLI registry. See [`[[agents]]`](./agents).                                          |
| `[profile]`      | singleton           | Picks the default session profile. See [`[profile]` & `[[profiles]]`](./profiles).          |
| `[[profiles]]`   | list, keyed by `id` | Session presets — at least one is required. See [`[profile]` & `[[profiles]]`](./profiles). |
| `[mcp]` / `mcps` | block / list        | Skills channel + MCP catalogue. See [`[mcp]` & `mcps`](./mcp).                              |
| `[[patches]]`    | list                | Partial-profile overlays. See [`[[patches]]`](./patches).                                   |
| `[multiplexer]`  | singleton           | tmux/zellij title rename. See [`[multiplexer]`](./multiplexer).                             |
| `[logging]`      | singleton           | Tracing filter level. See [`[logging]`](./logging).                                         |

There is **no** root-level `system_prompt` / `mcps` / `mcp` / `cwd` field — those are per-profile, or shared via `[[patches]]`.

## Validation

Every section validates types and rejects unknown fields at load — typos fail fast with an error naming the offending field path. Cross-field references are checked too: `[[profiles]].agent` must reference a real `[[agents]].id`, and `[profile] default` must name a real `[[profiles]].id`. The `[[profiles]]` list must be non-empty — a fresh install with no profile refuses to launch rather than guessing.
