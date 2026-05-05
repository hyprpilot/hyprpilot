import { computed, reactive, type ComputedRef } from 'vue'

import { nextSeq } from './sequence'
import { openTurnIdFor } from './use-turns'
import { useActiveInstance, type InstanceId } from '../chrome/use-active-instance'
import type { WireToolCall, WireToolCallContentBlock, WireToolCallLocation } from '@interfaces/ui'
import type { FormattedToolCall } from '@interfaces/wire/formatted-tool-call'

export type { WireToolCall, WireToolCallContentBlock, WireToolCallLocation }

export interface ToolsState {
  calls: WireToolCall[]
}

const states = reactive(new Map<InstanceId, ToolsState>())

function slotFor(id: InstanceId): ToolsState {
  let slot = states.get(id)

  if (!slot) {
    slot = { calls: [] }
    states.set(id, slot)
  }

  return slot
}

interface ToolCallUpdate {
  sessionUpdate: string
  toolCallId?: string
  title?: string
  status?: string
  kind?: string
  content?: WireToolCallContentBlock[]
  rawInput?: Record<string, unknown>
  locations?: WireToolCallLocation[]
  /// Daemon-authored presentation snapshot. Always present on
  /// `tool_call` and `tool_call_update` (the daemon recomputes
  /// against merged state per delta); UI replaces the stored value
  /// wholesale on each push.
  formatted: FormattedToolCall
  /// Wall-clock (epoch ms) of first observation; daemon stamps this
  /// on the cache miss and re-emits on every delta.
  startedAtMs: number
  /// Set on the first transition into Completed / Failed; absence
  /// = mid-flight (UI ticks live).
  completedAtMs?: number
}

export function pushToolCall(id: InstanceId, agentId: string, sessionId: string, raw: ToolCallUpdate): void {
  const slot = slotFor(id)
  const seq = nextSeq(id)
  const toolCallId = raw.toolCallId ?? `tc-${seq}`
  const existing = slot.calls.find((c) => c.toolCallId === toolCallId && c.sessionId === sessionId)

  if (existing) {
    existing.updatedAt = seq

    if (raw.title !== undefined) {
      existing.title = raw.title
    }

    if (raw.status !== undefined) {
      existing.status = raw.status
    }

    if (raw.kind !== undefined) {
      existing.kind = raw.kind
    }

    if (Array.isArray(raw.content)) {
      existing.content = raw.content
    }

    if (raw.rawInput !== undefined) {
      existing.rawInput = raw.rawInput
    }

    if (Array.isArray(raw.locations)) {
      existing.locations = raw.locations
    }
    existing.formatted = raw.formatted
    existing.startedAtMs = raw.startedAtMs

    if (raw.completedAtMs !== undefined) {
      existing.completedAtMs = raw.completedAtMs
    }

    return
  }
  slot.calls.push({
    id: `tc-${toolCallId}`,
    agentId,
    sessionId,
    turnId: openTurnIdFor(id, sessionId),
    toolCallId,
    title: raw.title,
    status: raw.status,
    kind: raw.kind,
    content: Array.isArray(raw.content) ? raw.content : [],
    rawInput: raw.rawInput,
    locations: Array.isArray(raw.locations) ? raw.locations : undefined,
    formatted: raw.formatted,
    startedAtMs: raw.startedAtMs,
    completedAtMs: raw.completedAtMs,
    createdAt: seq,
    updatedAt: seq
  })
}

export function resetTools(id: InstanceId): void {
  states.delete(id)
}

/** Drop every tool call tagged with `turnId`. Paired with
 * `deleteTurnByTurnId` in use-transcript to fully remove a
 * cancelled / errored turn from the visible chat. */
export function deleteToolsByTurnId(id: InstanceId, turnId: string): number {
  const slot = states.get(id)

  if (!slot) {
    return 0
  }
  const before = slot.calls.length

  slot.calls = slot.calls.filter((c) => c.turnId !== turnId)

  return before - slot.calls.length
}

export function getToolCall(id: InstanceId, toolCallId: string): WireToolCall | undefined {
  return states.get(id)?.calls.find((c) => c.toolCallId === toolCallId)
}

export function useTools(instanceId?: InstanceId): { calls: ComputedRef<WireToolCall[]> } {
  const { id: activeId } = useActiveInstance()
  const calls = computed<WireToolCall[]>(() => {
    const resolved = instanceId ?? activeId.value

    if (!resolved) {
      return []
    }

    return states.get(resolved)?.calls ?? []
  })

  return { calls }
}
