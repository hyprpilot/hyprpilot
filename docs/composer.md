# Composer autocomplete

The composer textarea has caret-anchored autocomplete with multiple
sources. Each source runs daemon-side; the UI just renders.

## Sources

| Sigil | Source | Trigger | Replacement |
| ----- | ------ | ------- | ----------- |
| `#` | skills | `#<query>` at word boundary | `#{skills://<slug>}` |
| `./` `~/` `/` | path | `./` / `~/` / `/<path>` at word boundary | absolute or relative path |
| `/` | command | `/<query>` at start of message | slash-command name |
| (manual) | ripgrep | `Tab` / `Ctrl+Space` (≥3 char prefix) | matched word from cwd / transcript |

The first source whose `detect()` matches owns the response. Walk
order: commands → skills → path → ripgrep.

## Keymap

| Key | Closed | Open |
| --- | ------ | ---- |
| sigil typed (`#`, `/`, `./`) | open + query | refine query |
| `Tab` | open + manual query | commit |
| `Ctrl+Space` | force-open + query | commit |
| `↑` / `↓` | (fall through) | navigate |
| `Enter` | (submit message) | commit |
| `Esc` | (fall through) | close |
| backspace past sigil | n/a | close |
| caret moves outside trigger range | n/a | close |
| textarea blurs | n/a | close |

## Skill tokens

Picking a skill inserts the literal token `#{skills://<slug>}` into the
textarea. On submit, `attachments_hydrate` scans the prompt text for
the URI pattern and hydrates each match into an `Attachment { slug,
path, body }` — the agent receives both the visible text and the
hydrated context. Unknown slugs silently drop with a daemon-side
`warn!` log.

## Wire surface

Three RPC methods, mirrored as Tauri commands:

- `completion/query` — fired per keystroke after 30ms debounce. Returns
  `{ requestId, sourceId, replacementRange, items }`.
- `completion/resolve` — fired 80ms after selection settles, lazy-loads
  documentation markdown.
- `completion/cancel` — fired when a newer query arrives mid-ripgrep.
  Other sources finish in sub-ms and ignore cancellation.

The UI tracks `latestQueryId`; older responses are dropped on receipt
without rendering. Daemon-side ripgrep holds a per-request
`Arc<AtomicBool>` cancel flag; the sink checks between matches and
halts the walk early.
