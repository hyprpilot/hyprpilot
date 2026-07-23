---
title: hyprpilot profiles
order: 20
---

# {{ $frontmatter.title }}

Lists configured session profiles without launching anything — the quickest way to check what a launch _would_ resolve.

<!-- more -->

```sh
hyprpilot profiles              # table: default marker, profile, agent, model
hyprpilot profiles --json       # machine-readable
```

The listing resolves config the same way a launch does, including root `[[patches]]`, so the displayed summaries reflect what you would actually get. It reads local config only — nothing is spawned.

## Flags

| Flag     | Purpose                                        |
| -------- | ---------------------------------------------- |
| `--json` | Emit machine-readable JSON instead of a table. |

The [global flags](./#global-flags) (`--config`, `--config-profile`, `--log-level`) apply as everywhere. `--json` keeps stdout pure — all tracing goes to stderr — so the output is safe to pipe into `jq`.

## Errors worth knowing

An empty `[[profiles]]` list is a validation error, not an empty table — fresh installs refuse to run until you configure at least one profile ([Quickstart](../guide/quickstart)). A config typo aborts with an error naming the offending field path.
