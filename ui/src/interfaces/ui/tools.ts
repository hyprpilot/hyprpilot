/**
 * Tool-call formatter contract. `format(call, adapter?)` (entry in
 * `lib/tools/index.ts`) routes a `WireToolCall` (the per-instance
 * tool-call record streamed off `acp:transcript`) through the right
 * per-tool formatter and produces a `ToolCallView` every consumer
 * (chat pill, permission row, plan-mode modal) reads off.
 */

import type { IconDefinition } from '@fortawesome/fontawesome-svg-core'

import type { AdapterId, PermissionUi, PillKind, ToolState, ToolType } from '@constants/ui'

/** Untyped arg map flowing through formatters. */
export type Args = Record<string, unknown>

/** Single key/value row surfaced under the spec sheet's structured fields. */
export interface ToolField {
  /// Lowercase short label rendered as the row prefix
  /// ("path" / "pattern" / "tool"). Stays uniform with the spec
  /// sheet's hardcoded rows.
  label: string
  /// Free-form value. Long values wrap; mono-font.
  value: string
}

/**
 * Single unified result type every formatter produces. Drives every
 * consumer:
 *
 *  - chat pill (`views/chat/ToolPill.vue`) reads `{ icon, title,
 *    stat, state, type, description, output, fields }`.
 *  - permission row (`views/composer/PermissionRow.vue`) reads
 *    `{ icon, title, fields }`.
 *  - permission modal (`views/chat/PermissionModal.vue`) reads
 *    `{ title, description, fields, output }`.
 *
 * Pill rendering is a 3-section layout: `[icon] [title] [stat]` —
 * the formatter composes `title` from whatever pieces it has (path
 * + range, command + descriptor, etc.). No per-section breakdown
 * lives in this shape.
 */
export interface ToolCallView {
  /// `tool_call.id` from the wire — stable identity for the tool.
  id: string
  /// Formatter-output discriminator. Drives any downstream
  /// type-aware logic; the icon is supplied directly via `icon`.
  type: ToolType
  /// Raw wire tool name ("Bash" / "mcp__playwright__browser_navigate").
  /// Trust-store keying + permission glob matching read this verbatim.
  name: string
  /// Lifecycle state — drives the pill's tone tint.
  state: ToolState

  /// FontAwesome icon, formatter-supplied. The chat pill renders
  /// `<FaIcon :icon="view.icon" />` directly.
  icon: IconDefinition
  /// Pill-style discriminator. Only one variant today; future
  /// shapes slot in as additional members.
  pill: PillKind
  /// Permission-flow surface declaration. `Row` for nearly every
  /// tool; `Modal` reserved for plan-exit and other heavy flows.
  permissionUi: PermissionUi

  /// Composed display string for the pill's center cell. The
  /// formatter assembles "Bash · npm test", "Edit auth.ts (replace
  /// all)", etc. — consumers don't splice fragments.
  title: string
  /// Optional pill-right-cell metric ("1.4s", "2 edits", "11
  /// chars"). Short by convention.
  stat?: string

  /// Markdown body. Always rendered AS markdown by every consumer
  /// (chat pill expanded body, modal body). Formatters only assign
  /// when the source is markdown-shaped — when in doubt, route the
  /// content to `output` instead.
  description?: string
  /// Tool execution result rendered as preformatted plain text
  /// (stdout / diff / file content).
  output?: string
  /// Structured key/value rows for MCP arg dumps + arbitrary JSON.
  fields?: ToolField[]

  /// Raw `tool_call.rawInput` JSON pass-through — needed by the
  /// permission flow for trust-store keying alongside `name`.
  rawInput?: Record<string, unknown>
}

/**
 * Wire tool call as stored by `composables/instance/use-tools` after
 * receiving `acp:transcript` events. The formatter consumes this.
 */
export interface WireToolCallContentBlock {
  [k: string]: unknown
  type?: string
  text?: string
}

export interface WireToolCallLocation {
  path?: string
  line?: number
}

export interface WireToolCall {
  id: string
  sessionId: string
  /// Active ACP turn id at first-sight; preserved across subsequent
  /// `tool_call_update` chunks for the same `toolCallId`.
  turnId?: string
  toolCallId: string
  title?: string
  status?: string
  kind?: string
  content: WireToolCallContentBlock[]
  rawInput?: Record<string, unknown>
  locations?: WireToolCallLocation[]
  createdAt: number
  updatedAt: number
}

/**
 * Per-formatter context. The formatter has the raw wire payload
 * available via `raw` if it needs more than the normalised fields.
 */
export interface FormatterContext {
  /// Raw wire tool name.
  name: string
  /// Normalised arg map (every key lowercased and underscore-stripped
  /// per `normaliseArgs`).
  args: Args
  /// Lifecycle state derived from `wire.status`.
  state: ToolState
  /// The full wire payload. Formatters that need to read content
  /// blocks, locations, or other ancillary fields reach for this.
  raw: WireToolCall
  /// Adapter that emitted the call. Formatters may branch internally
  /// when divergence is small; larger divergences live in per-adapter
  /// override files.
  adapter?: AdapterId
}

/** A per-tool formatter — one shared `fallback` + optional per-adapter overrides. */
export interface Formatter {
  type: ToolType
  format: (ctx: FormatterContext) => ToolCallView
}

/** Resolves a `Formatter` for a given adapter. */
export type Formatters = (adapter?: AdapterId) => Formatter

/**
 * Build a `Formatters` resolver for a tool with optional per-adapter
 * overrides. Each per-tool `index.ts` calls this with its `fallback`
 * formatter + a (possibly empty) overrides map.
 */
export function pickFormatter(fallback: Formatter, overrides: Partial<Record<AdapterId, Formatter>> = {}): Formatters {
  return (adapter) => (adapter ? (overrides[adapter] ?? fallback) : fallback)
}
