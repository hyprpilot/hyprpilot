/**
 * Top-level barrel for typed shapes. Two axes:
 *
 *  - `wire/` — Tauri / IPC contract types mirroring Rust shapes.
 *  - `ui/` — UI-only types feature-grouped (chat, composer, palette,
 *    header, keyboard).
 *
 * Consumers import from this barrel: `import type { Theme,
 * SessionRow } from '@interfaces'`. Sub-barrel imports are fine when
 * a file only needs one axis: `import type { Theme } from
 * '@interfaces/wire'`.
 */
export * from './wire'
export * from './ui'
