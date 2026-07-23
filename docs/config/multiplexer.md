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

## Best-effort by design

The rename never gets in the way of a launch:

- Outside tmux/zellij it is a no-op, regardless of the flag.
- Any failure (missing `tmux` binary, a denied action) is logged at `debug` and never aborts the launch.

Because hyprpilot `exec()`s away immediately after, the title is a one-shot stamp — nothing keeps it updated afterwards, and your multiplexer's own automatic-rename settings take over as usual once the vendor exits.
