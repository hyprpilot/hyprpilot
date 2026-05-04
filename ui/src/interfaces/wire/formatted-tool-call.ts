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

export interface FormattedToolCall {
  title: string
  stat?: string
  description?: string
  output?: string
  fields: ToolField[]
}
