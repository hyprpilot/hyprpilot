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
export type Stat =
  | { kind: 'text'; value: string }
  | { kind: 'diff'; added: number; removed: number }
  | { kind: 'duration'; ms: number }
  | { kind: 'matches'; count: number }

export interface FormattedToolCall {
  title: string
  stats: Stat[]
  description?: string
  output?: string
  fields: ToolField[]
}
