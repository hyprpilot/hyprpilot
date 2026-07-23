---
title: '[logging]'
order: 60
next: false
---

# {{ $frontmatter.title }}

The tracing filter applied when no higher-precedence source speaks. Narrative: [Features → Logging](../features/logging).

<!-- more -->

```toml
[logging]
level = "info"
```

| Field   | Type | Default | What it does                                                                                                         |
| ------- | ---- | ------- | -------------------------------------------------------------------------------------------------------------------- |
| `level` | enum | `info`  | One of `trace` / `debug` / `info` / `warn` / `error`. Applied only when `--log-level` and `RUST_LOG` are both unset. |

Full precedence: `--log-level` → `RUST_LOG` → `[logging] level` → the built-in `warn,hyprpilot=info` default. Logs always go to stderr.
