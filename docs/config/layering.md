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
4. **Directory layers** — `.hyprpilot.{ext}` and `.hyprpilot.<overlay>.{ext}` files found walking up from the launch directory, only in folders `project_config.trusted` covers. See [Directory layers](#directory-layers).
5. **`patches` and `--with-config`** — profile overlays applied at resolve time. See [Patches](./patches) and [Ad-hoc Overlays](../runtime/with-config).

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

## Directory layers

A folder can carry its own config: a `.hyprpilot.{toml,json,yaml,yml}` file, plus `.hyprpilot.<overlay>.{ext}` files that load only under that `--config-profile`. Use them for what one project needs and nothing else does, such as an MCP server for one employer's code host.

```yaml
# ~/development/acme/.hyprpilot.yaml
patches:
  - mcps:
      - mcpServers:
          acme-git:
            url: https://mcp.example.com/mcp
            headers:
              Authorization: 'Bearer ${ACME_TOKEN}'
```

**Discovery.** Hyprpilot walks from the launch directory (`--cwd` when given, otherwise the current directory) up to `/`, and folds every file it finds **outermost folder first**, so the folder closest to the launch wins. Within one folder the plain `.hyprpilot.*` file folds first, then one `.hyprpilot.<overlay>.*` per active overlay in `--config-profile` order. The [Agent Harness](../runtime/harness) walks from the session's own `cwd`, so a delegate gets its folder's layers.

**Trust.** A directory layer can add stdio MCP servers and agent commands, so hyprpilot reads one only when its folder matches a glob in `project_config.trusted`. That list is read from the global config and the overlays only; a directory layer that sets `project_config` fails the load, so a folder can never trust itself. A file in an untrusted folder is skipped with one warning naming it.

```yaml
# ~/.config/hyprpilot/config.yaml
project_config:
  trusted:
    - ~/development/acme
    - ~/development/acme/**
```

Globs match the folder path the same way `$match.profile` matches ids: `*` crosses `/`, `~` and `${VAR}` expand first. `~/development/acme/**` covers everything **below** `acme` but not `acme` itself, so list both. With no `trusted` entries there is no walk at all.

**Merge.** A directory layer is an ordinary layer with the [merge rules](#merge-rules) below, and may set every section except `project_config`. Its `patches` append after the overlays' patches, so an unscoped one reaches every profile, including those an overlay patch meant to keep bare; scope it with `$match` when that matters. Two formats of one file in the same folder (`.hyprpilot.toml` beside `.hyprpilot.yaml`) fail the load, as they do for the global config.

`hyprpilot --log-level debug profiles` names every directory layer it loaded.

## Validation

After the layers merge, the whole config is validated in one pass:

- **Unknown fields reject at parse time** — every section is a closed shape, so a typo like `modle: …` fails with an error naming the field.
- **Closed sets are enums** — `provider`, `--log-level` values, and config formats reject unknown values at parse, not deep in a launch.
- **Cross-field references are checked** — `profiles[].agent` must reference a real `agents[].id`, and `profile.default` must name a real `profiles[].id`.
- **The `profiles` list must be non-empty** — a fresh install with no profile refuses to launch rather than guessing.

Validation failures abort startup with a readable field-path error, so a broken config never reaches the vendor CLI.

## Where things live

| Path                                                | What                                    |
| --------------------------------------------------- | --------------------------------------- |
| `~/.config/hyprpilot/config.{toml,json,yaml,yml}`   | Global config.                          |
| `~/.config/hyprpilot/profiles/*.{ext}`              | Named config-layer overlays.            |
| `<any trusted folder>/.hyprpilot[.<overlay>].{ext}` | Directory layers.                       |
| `~/.config/hyprpilot/skills/<path>/SKILL.md`        | Skill bundles (default catalogue root). |
| `~/.config/hyprpilot/mcps/*.json`                   | MCP catalogue files (your convention).  |
