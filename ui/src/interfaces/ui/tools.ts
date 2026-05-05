/**
 * UI-side tool-call view. The daemon emits `FormattedToolCall`
 * (rendering content); the UI layers `Presentation` (icon + pill +
 * permissionUi) per-(kind, adapter, wireName) via
 * `lib/tools/presentation.ts`. `ToolCallView` is the unified shape
 * every consumer (chat pill, permission row, modal) reads.
 */

import type { IconDefinition } from '@fortawesome/fontawesome-svg-core'

import type { PermissionUi, PillKind, ToolKind, ToolState } from '@constants/ui'
import type { FormattedToolCall, Stat, ToolField as WireToolField } from '@interfaces/wire/formatted-tool-call'

export type { ToolField } from '@interfaces/wire/formatted-tool-call'

export interface ToolCallView {
  /// `tool_call.id` from the wire — stable identity for the tool.
  id: string
  /// ACP `tool_call.kind` classification.
  kind: ToolKind
  /// Raw wire tool name ("Bash", "mcp__playwright__browser_navigate").
  /// Trust-store keying + permission glob matching read this verbatim.
  name: string
  /// UI tone state — derived from the wire raw `ToolCallState`.
  state: ToolState
  /// Resolved FontAwesome icon (looked up from
  /// `(kind, adapter, wireName)` via `presentationFor`).
  icon: IconDefinition
  pill: PillKind
  permissionUi: PermissionUi
  title: string
  stats: Stat[]
  description?: string
  output?: string
  fields: WireToolField[]
  /// Raw `tool_call.rawInput` pass-through. Permission flow uses it
  /// for trust-store keying alongside `name`.
  rawInput?: Record<string, unknown>
}

/**
 * Wire tool call as stored by `composables/instance/use-tools` after
 * receiving `acp:transcript` events.
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
  /// `acp:transcript` event's `agentId` (config-defined name like
  /// `claude-code`). Threaded through `pushToolCall` so the
  /// presentation layer can resolve `agentId → AdapterId` without
  /// extra lookups at render time.
  agentId: string
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
  /// Daemon-authored presentation content. Re-emitted on every
  /// `tool_call_update` against merged running state.
  formatted: FormattedToolCall
  /// Daemon-stamped wall-clock (epoch ms). Stable across updates.
  startedAtMs: number
  /// Set when the call transitions into Completed / Failed; absence
  /// = mid-flight, UI ticks elapsed labels live.
  completedAtMs?: number
  createdAt: number
  updatedAt: number
}
