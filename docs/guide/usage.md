---
title: Everyday Usage
order: 30
next: false
---

# {{ $frontmatter.title }}

The bare invocation **is** the launch — there is no `run` subcommand. Everything below tunes what that launch resolves; the full flag tables live in the [CLI reference](../cli/).

<!-- more -->

## Picking a profile

```sh
hyprpilot                       # interactive picker over configured profiles
hyprpilot -p engineer           # launch the `engineer` profile directly
```

If you omit `--profile`/`-p` and no `[profile] default` is set, an interactive fuzzy picker (powered by `nucleo`) opens over your configured profiles — each row shows the default marker, id, agent, model, and cwd. Cancelling the picker aborts the launch; a non-interactive terminal errors instead of hanging.

## Overriding per launch

If you want to deviate from the profile for one launch, the launch flags override the resolved profile without touching your config:

```sh
hyprpilot -p engineer --agent codex           # swap the agent entry wholesale
hyprpilot -p engineer --cwd ~/code/foo        # run somewhere else
hyprpilot -p engineer --model claude-opus-4-5 # different model
hyprpilot -p engineer --mode plan             # vendor-specific mode
```

- `--agent <id>` swaps which `[[agents]]` entry the launch uses — it wins over whatever agent the (patched) profile names.
- `--cwd <dir>` beats the profile's (or agent's) configured `cwd`; when neither speaks, the current directory is used.
- `--model` / `--mode` are projected onto the vendor CLI where supported.

For structural one-off overrides — a different MCP set, an extra system prompt — reach for [`--with-config`](../features/with-config) instead.

## Forwarding native arguments

Everything after a `--` separator is forwarded verbatim to the vendor CLI — use it for provider-native flags and resume flows:

```sh
hyprpilot -p engineer -- --resume
hyprpilot -p review -- --dangerously-skip-permissions
```

Any provider-native argument you pass this way suppresses hyprpilot's generated equivalent, so your hand-written flag always wins over the projection.

## Environment twins

Every global flag has an environment twin, so you can pin them per shell or per session:

| Flag               | Environment variable       |
| ------------------ | -------------------------- |
| `--config <path>`  | `HYPRPILOT_CONFIG`         |
| `--config-profile` | `HYPRPILOT_CONFIG_PROFILE` |
| `--log-level`      | `HYPRPILOT_LOG_LEVEL`      |

```sh
HYPRPILOT_CONFIG_PROFILE=work hyprpilot -p engineer
```

`--config-profile` layers a named config file (`~/.config/hyprpilot/profiles/<name>.toml`) on top of your global config — a **config-layer** profile, distinct from the session `[[profiles]]` you address with `-p`. See [Config Layering](../features/layering).

## Inspecting without launching

```sh
hyprpilot profiles              # table of configured profiles
hyprpilot profiles --json       # machine-readable
```

`profiles` resolves config the same way a launch does (including root `[[patches]]`) but stops before exec — handy for checking what a launch _would_ use.
