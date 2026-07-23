---
title: '[multiplexer]'
order: 50
---

# {{ $frontmatter.title }}

Best-effort tmux/zellij window-title rename before exec. Narrative: [Features → Multiplexer Title](../features/multiplexer).

<!-- more -->

```toml
[multiplexer]
set_title = true
```

| Field       | Type | Default | What it does                                                                                                                                                                      |
| ----------- | ---- | ------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `set_title` | bool | `true`  | Rename the current tmux window / zellij tab to `hyprpilot@<cwd-basename>` right before `exec()`. No-op outside a multiplexer; failures log at `debug` and never abort the launch. |
