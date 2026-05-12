/**
 * Daemon-authored tool-call presentation content. Mirrors
 * `src-tauri/src/tools/formatter/types::FormattedToolCall`.
 *
 * The wire shape is **rendering content only** — `state` and `kind`
 * already live on the parent `ToolCallRecord`, and presentation
 * chrome (icon / pill style / permission-flow surface) is the
 * frontend's call. Each frontend resolves
 * `(toolKind, adapter, wireName) → { icon, pill, permissionUi }`
 * locally; in this codebase that lives in
 * `ui/src/lib/tools/presentation.ts`.
 */

export interface ToolField {
  label: string
  value: string
}

/**
 * Per-stat-pill rendering content. Tagged enum mirrors the Rust
 * `Stat` enum (serde `tag = "kind"`, snake-case variant rename).
 * Empty `stats` vec on `FormattedToolCall` = no pills rendered.
 */
export type Stat = { kind: 'text'; value: string } | { kind: 'diff'; added: number; removed: number } | { kind: 'duration'; ms: number }

export interface FormattedToolCall {
  title: string
  stats: Stat[]
  description?: string
  /**
   * Plain unified git-diff patch (`--- a/path` + `+++ b/path` +
   * `@@ -1,N +1,M @@` + `+/-` lines). Parallel to `description`
   * (which is Shiki-marker markdown for the desktop overlay); this
   * field is for consumers that can't drive the Shiki transformer
   * pipeline — Neovim, plain markdown renderers, GitHub paste,
   * `patch -p1`-style tooling. `undefined` when the tool doesn't
   * produce a diff (read / glob / bash / …).
   */
  diff?: string
  output?: string
  fields: ToolField[]
}
