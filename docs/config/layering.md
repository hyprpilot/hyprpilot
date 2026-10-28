---
title: Layering
order: 10
---

# {{ $frontmatter.title }}

Hyprpilot reads layered config. Each source overrides the one before it for the fields it sets, so you only write what you want to change.

<!-- more -->

## The layers

1. **Compiled defaults** — every knob has a working default, baked into the binary from `src/config/defaults.toml`.
2. **Global config** — `~/.config/hyprpilot/config.{toml,json,yaml,yml}`, or an explicit `--config <path>`.
3. **Config-layer overlays** — `~/.config/hyprpilot/profiles/<name>.{ext}`, picked with `--config-profile <name>` (repeatable) or `HYPRPILOT_CONFIG_PROFILE=<name>[,<name>…]`. Overlays fold in the order given, each on top of the one before; flags on the command line replace the env list rather than extending it.
4. **`patches` and `--with-config`** — profile overlays applied at resolve time. See [Patches](./patches) and [Ad-hoc Overlays](../runtime/with-config).

Any layer can be any supported format — a YAML global config composes with a TOML config-layer profile. See [Formats](./#formats) for extension discovery.

## Merge rules

- **Scalar fields overwrite** — a later layer's value wins for the fields it sets.
- **The keyed `agents` / `profiles` lists merge by `id`** — a later layer's entry with a matching `id` replaces the earlier entry wholesale, and new ids append. There is no field-level merge inside a single entry.
- **The `patches` list is additive** — each layer's patches **append** to the earlier layers' list instead of replacing it, so the seeded default patch and your global patches survive a config-layer profile that adds more. See [Patches → Additive across layers](./patches#additive-across-layers) for how to neutralize an inherited patch.

Overlays follow the same rules among themselves, so with `--config-profile personal --config-profile work` a `profiles` entry both files define comes from `work`, and `work`'s patches fold after `personal`'s.

`~` and `${VAR}` / `${env:VAR}` in path-valued fields expand at consume time; relative paths resolve against the current directory.

## Config-layer profile ≠ session profile

The word "profile" lives in two parallel namespaces — keep them apart:

| Concept              | Addressed via                                               | Purpose                                                                                     |
| -------------------- | ----------------------------------------------------------- | ------------------------------------------------------------------------------------------- |
| Config-layer profile | `--config-profile` / `HYPRPILOT_CONFIG_PROFILE`             | Layer a different config file overlay (e.g. `work` vs `personal`).                          |
| Session profile      | `profiles` in config, picked via the positional `[PROFILE]` | Which agent + model + cwd + system prompt + MCPs a launch uses. See [Profiles](./profiles). |

A config-layer profile can itself define or override session profiles — that is the point: `HYPRPILOT_CONFIG_PROFILE=work` can swap your whole `profiles` registry. Two accounts can therefore keep identical session ids (`claude/opus` in both `personal.yaml` and `work.yaml`), and the overlay decides which one a launch gets:

```sh
hyprpilot --config-profile work claude/opus      # the work account's claude/opus
hyprpilot --config-profile personal claude/opus  # the personal one
```

The selection follows the launch into the [Agent Harness](../runtime/harness): the injected `hyprpilot mcp harness` entry carries `--config` and every `--config-profile` explicitly, so its `list_profiles` / `spawn` read the same layers even when the vendor filters the environment it hands MCP servers, and the sessions it spawns pass the selection on again.

## Validation

After the layers merge, the whole config is validated in one pass:

- **Unknown fields reject at parse time** — every section is a closed shape, so a typo like `modle: …` fails with an error naming the field.
- **Closed sets are enums** — `provider`, `--log-level` values, and config formats reject unknown values at parse, not deep in a launch.
- **Cross-field references are checked** — `profiles[].agent` must reference a real `agents[].id`, and `profile.default` must name a real `profiles[].id`.
- **The `profiles` list must be non-empty** — a fresh install with no profile refuses to launch rather than guessing.

Validation failures abort startup with a readable field-path error, so a broken config never reaches the vendor CLI.

## Where things live

| Path                                              | What                                    |
| ------------------------------------------------- | --------------------------------------- |
| `~/.config/hyprpilot/config.{toml,json,yaml,yml}` | Global config.                          |
| `~/.config/hyprpilot/profiles/*.{ext}`            | Named config-layer overlays.            |
| `~/.config/hyprpilot/skills/<path>/SKILL.md`      | Skill bundles (default catalogue root). |
| `~/.config/hyprpilot/mcps/*.json`                 | MCP catalogue files (your convention).  |
