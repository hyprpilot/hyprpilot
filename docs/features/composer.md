---
title: Composer
order: 4
---

# Composer

The composer is the textarea at the bottom of the overlay. It accepts plain text, image attachments, skill attachments, and offers caret-anchored autocomplete from multiple sources.

![composer mid-type with autocomplete popover open](/screenshots/composer-autocomplete.png)

## Autocomplete sources

| Sigil | Source | Trigger | Result |
| --- | --- | --- | --- |
| `#` | Skills | `#<query>` at word boundary | Picks a skill, attaches its body to the next prompt as an embedded resource. |
| `./` `~/` `/` | Path | `./` / `~/` / `/<path>` at word boundary | Inserts the resolved absolute path. |
| `/` | Command | `/<query>` at start of message | Slash command (e.g. `/clear`). |
| (manual) | Ripgrep | `Tab` or `Ctrl+Space`, ≥3-char prefix | Inserts a matching word from cwd files / current transcript. |

The first source whose detector matches owns the response — no overlap.

## Keymap

| Key | Closed | Open |
| --- | --- | --- |
| Type a sigil (`#`, `/`, `./`) | Opens with that source | Refines query |
| `Tab` | Opens manual query | Commits highlighted row |
| `Ctrl+Space` | Force-opens at caret | Commits highlighted row |
| `↑` / `↓` | (falls through to history) | Navigate rows |
| `Enter` | Submits the message | Commits highlighted row |
| `Esc` | (falls through) | Closes popover |
| Backspace past the sigil | n/a | Closes popover |

## Other inputs

- **Image attachments.** Drag image files from the OS, or `Ctrl+P` to paste from the clipboard. The composer encodes them inline as PNG and attaches them as `ContentBlock::Image` on the next prompt.
- **Skill attachments.** Picked via `Ctrl+K → skills` (or the `#<query>` autocomplete sigil). The skill body snapshot rides as `ContentBlock::Resource` in front of your text — the agent reads context first, then your instructions.
- **History.** `↑` / `↓` walks previously-submitted prompts when the composer is empty.

## Submit + queue

`Enter` sends the prompt to the focused instance. If a turn is already in flight, the prompt **queues** above the composer — `Ctrl+Enter` dispatches the head; per-row edit / delete buttons reshape items before send. The queue drains as turns complete.

## What runs daemon-side

Path resolution, skill lookup, ripgrep walks all run in the Rust daemon. The UI just emits `completion_query` and renders the result. Ripgrep walks honor a 30 ms debounce so fast keystrokes don't pile up requests; an in-flight walk cancels when a newer query arrives.

If you need to tune ripgrep behaviour:

```toml
[completion.ripgrep]
auto = true              # auto-trigger on typing
debounceMs = 80          # wait this long before firing
minPrefix = 3            # don't trigger on 1-2 char queries
```
