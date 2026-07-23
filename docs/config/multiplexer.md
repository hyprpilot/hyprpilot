---
title: Multiplexer
order: 60
---

# {{ $frontmatter.title }}

When hyprpilot launches inside tmux or zellij, it renames the current window / tab to `hyprpilot@<cwd-basename>` right before `exec()` — so you can tell agent panes apart at a glance.

<!-- more -->

## Configuration

It is on by default:

```yaml
multiplexer:
  set_title: true # default; set false to opt out
```

| Field       | Type | Default | What it does                                                                           |
| ----------- | ---- | ------- | -------------------------------------------------------------------------------------- |
| `set_title` | bool | `true`  | Rename the current tmux window / zellij tab to `hyprpilot@<cwd-basename>` before exec. |

## How it renames

The rename shells out to the multiplexer's own CLI, not raw OSC escape sequences — those are gated by tmux/zellij settings, the CLI is not:

::: code-group

```sh [tmux]
tmux rename-window 'hyprpilot@my-project'
```

```sh [zellij]
zellij action rename-tab 'hyprpilot@my-project'
```

:::

The `<cwd-basename>` is the base name of the **resolved** working directory — after `--cwd`, the profile's `cwd`, and the current-directory fallback have been applied.

## When the rename is skipped

The rename proceeds only when **all** of these hold — any one skips it (logged at `debug`, never aborting the launch):

- `set_title` is not `false`.
- `HYPRPILOT_NO_TITLE` is unset or falsey.
- hyprpilot is not running under an editor.

### `HYPRPILOT_NO_TITLE` — the explicit override

Set `HYPRPILOT_NO_TITLE` to a truthy value (`1`, `true`, or any non-empty value other than `0` / `false`) to skip the rename unconditionally — independent of `set_title` and of editor auto-detection. This is the authoritative hook a launcher sets in the environment when it owns the pane itself (for example an nvim plugin like `sidekick.nvim` that sets a per-tool `env` block). Because `[multiplexer]` is a **root** config field, it can't be reached via `--with-config` (which patches the profile) — the env var is the right hook.

### Editor auto-skip

When hyprpilot is spawned as a child job/terminal of an editor, that editor owns the multiplexer pane, so renaming it from underneath is wrong. hyprpilot auto-detects this from environment markers and skips the rename without needing `HYPRPILOT_NO_TITLE`:

| Marker                               | Editor                 |
| ------------------------------------ | ---------------------- |
| `NVIM`                               | nvim ≥ 0.5             |
| `NVIM_LISTEN_ADDRESS`                | older nvim             |
| `INSIDE_EMACS`                       | Emacs                  |
| `VSCODE_PID` / `TERM_PROGRAM=vscode` | VS Code                |
| `VIM`                                | vim (lower confidence) |

## Best-effort by design

The rename never gets in the way of a launch:

- Outside tmux/zellij it is a no-op, regardless of the flag.
- Any failure (missing `tmux` binary, a denied action) is logged at `debug` and never aborts the launch.

Because hyprpilot `exec()`s away immediately after, the title is a one-shot stamp — nothing keeps it updated afterwards, and your multiplexer's own automatic-rename settings take over as usual once the vendor exits.
