/**
 * Tool-call view facade. The daemon emits `FormattedToolCall`
 * (rendering content); the UI layers `Presentation` (chrome) via
 * `presentation.ts`. `format()` stitches both into the unified
 * `ToolCallView` every consumer (chat pill, permission row, modal)
 * reads.
 */

import { presentationFor } from './presentation'
import { AdapterId, ToolKind, toolStateFromWire } from '@constants/ui'
import type { ToolCallState } from '@constants/wire/transcript'
import type { ToolCallView, WireToolCall } from '@interfaces/ui'
import type { FormattedToolCall } from '@interfaces/wire/formatted-tool-call'

export { presentationFor }
export type { Presentation } from './presentation'

export function format(call: WireToolCall, adapter?: AdapterId): ToolCallView {
  return projectFormatted(call.formatted, {
    id: call.id,
    wireName: call.title ?? '',
    kind: (call.kind as ToolKind | undefined) ?? ToolKind.Other,
    state: call.status as ToolCallState | undefined,
    adapter,
    rawInput: call.rawInput
  })
}

export interface ProjectionMeta {
  id: string
  wireName: string
  kind: ToolKind
  state: ToolCallState | undefined
  adapter: AdapterId | undefined
  rawInput?: Record<string, unknown>
}

/**
 * Project a daemon-authored `FormattedToolCall` + classification
 * triplet (kind, adapter, wireName) onto the UI's `ToolCallView`.
 * Pulls presentation chrome via `presentationFor()`; pulls UI tone
 * `ToolState` from the wire `ToolCallState` via `toolStateFromWire`.
 */
export function projectFormatted(formatted: FormattedToolCall, meta: ProjectionMeta): ToolCallView {
  const presentation = presentationFor(meta.kind, meta.adapter, meta.wireName, meta.rawInput)

  return {
    id: meta.id,
    kind: meta.kind,
    name: meta.wireName,
    state: toolStateFromWire(meta.state),
    icon: presentation.icon,
    pill: presentation.pill,
    permissionUi: presentation.permissionUi,
    title: formatted.title,
    stat: formatted.stat,
    description: formatted.description,
    output: formatted.output,
    fields: formatted.fields,
    rawInput: meta.rawInput
  }
}
