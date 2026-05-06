# Docs screenshots

Reproducible 2560×1440 dark-mode screenshots for the documentation site.

## Pipeline

`run-all.ts` is a Playwright orchestrator that:

1. Launches Chromium at 2560×1440.
2. Navigates to the Vite dev preview (`localhost:1420` by default).
3. For each of 9 screenshots, calls `window.__hyprpilot_dev.*` to seed mock daemon state.
4. Captures the result to `docs/public/screenshots/<name>.png`.

The seed scripts mirror real claude-code/haiku state without needing a live daemon — every push helper (`pushSessionInfoUpdate`, `pushToolCall`, `pushPermissionRequest`, …) is exposed by `tests/dev-preview.ts`.

## Capturing

Run two commands:

```sh
# Terminal 1 — dev preview with the dev-preview shim active
pnpm --filter hyprpilot-ui dev

# Terminal 2 — capture all 9 shots
pnpm --filter hyprpilot-docs run screenshots
```

Or via task targets:

```sh
task dev          # in one terminal — boots Tauri + Vite
# kill the Tauri side, leave Vite running on :1420
task docs:screenshots
```

## Inventory

| File | Page | Seeded state |
| --- | --- | --- |
| `hero.png` | `index.md` | Active turn — captain prompt + agent thinking. |
| `idle-screen.png` | `index.md` (or `guide/installation.md`) | Empty state — LFG accent + recent-sessions preview. |
| `palette-root.png` | `features/command-palette.md` | `Ctrl+K` root, all 11 leaves visible. |
| `palette-sessions.png` | `features/command-palette.md` | Sessions leaf, cwd-filtered. |
| `palette-models.png` | `features/command-palette.md` | Models leaf with claude-haiku highlighted. |
| `chat-tool-pills.png` | `features/chat-and-tools.md` | Transcript with bash + edit + read pills. |
| `permission-modal.png` | `features/chat-and-tools.md` | Permission request for `bash rm -rf`. |
| `composer-autocomplete.png` | `features/composer.md` | Composer mid-type with autocomplete popover. |
| `multi-instance-header.png` | `features/chat-and-tools.md` | Header chrome showing 3 instances. |

## Updating

When the UI changes:

1. Run the pipeline (above).
2. Diff the resulting PNGs in `docs/public/screenshots/`.
3. Commit changes with a `docs(screenshots): refresh after <feature>` message.

The seed scripts are self-contained — anyone can re-run them without bespoke setup. If a UI change makes a seed obsolete, update the relevant `seed:` block in `run-all.ts`.
