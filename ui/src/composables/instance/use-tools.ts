import { computed, markRaw, reactive, type ComputedRef } from 'vue'

import { nextSeq } from './sequence'
import { openTurnIdFor } from './use-turns'
import { useActiveInstance, type InstanceId } from '../chrome/use-active-instance'
import type { WireToolCall, WireToolCallContentBlock, WireToolCallLocation } from '@interfaces/ui'
import type { FormattedToolCall } from '@interfaces/wire/formatted-tool-call'
import type { ToolIdentity } from '@interfaces/wire/transcript'

export type { WireToolCall, WireToolCallContentBlock, WireToolCallLocation }

export interface ToolsState {
  calls: WireToolCall[]
  /// O(1) counter of calls whose `status` is non-terminal. Maintained
  /// inline with mutation so `usePhase` can answer
  /// "is anything running?" without scanning `calls` per chunk.
  runningCount: number
}

const TERMINAL_STATUSES = new Set(['completed', 'done', 'failed', 'error'])

function isRunning(status: string | undefined): boolean {
  if (!status) {
    return true
  }

  return !TERMINAL_STATUSES.has(status.toLowerCase())
}

const states = reactive(new Map<InstanceId, ToolsState>())

/// `formatted`, `content`, `rawInput` carry diff blobs / large
/// command outputs / argument trees. Wrapping them in `markRaw`
/// before they hit the reactive slot stops Vue's reactivity proxy
/// from deep-traversing every read — every render of the tool pill
/// otherwise walks the entire diff structure to set up tracking.
/// Each wrap is guarded against `undefined`/null because some test
/// fixtures and edge wire shapes omit these fields entirely; Vue's
/// `markRaw` does not accept primitives.
function rawifyMaybe<T>(value: T): T {
  if (value !== null && typeof value === 'object') {
    return markRaw(value as object) as T
  }

  return value
}

function rawifyHeavyFields(raw: ToolCallUpdate): ToolCallUpdate {
  return {
    ...raw,
    formatted: rawifyMaybe(raw.formatted),
    content: Array.isArray(raw.content) ? rawifyMaybe(raw.content) : raw.content,
    rawInput: rawifyMaybe(raw.rawInput),
    locations: Array.isArray(raw.locations) ? rawifyMaybe(raw.locations) : raw.locations
  }
}

function recountRunning(slot: ToolsState): void {
  let n = 0

  for (const c of slot.calls) {
    if (isRunning(c.status)) {
      n += 1
    }
  }
  slot.runningCount = n
}

function slotFor(id: InstanceId): ToolsState {
  let slot = states.get(id)

  if (!slot) {
    slot = { calls: [], runningCount: 0 }
    states.set(id, slot)
  }

  return slot
}

interface ToolCallUpdate {
  sessionUpdate: string
  toolCallId?: string
  identity?: ToolIdentity
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

// ── Internal store-mutation surface ───────────────────────────────
// Sibling-store wire-listener inputs. Per-feature views read via
// `useTools()`; the wire router pushes through these free fns
// directly. CLAUDE.md "Two-tier composables" documents the convention.

export function pushToolCall(id: InstanceId, agentId: string, sessionId: string, raw: ToolCallUpdate): void {
  const slot = slotFor(id)
  const seq = nextSeq(id)
  const toolCallId = raw.toolCallId ?? `tc-${seq}`
  const heavy = rawifyHeavyFields(raw)
  const existing = slot.calls.find((c) => c.toolCallId === toolCallId && c.sessionId === sessionId)

  if (existing) {
    existing.updatedAt = seq

    if (heavy.title !== undefined) {
      existing.title = heavy.title
    }

    if (heavy.status !== undefined) {
      const wasRunning = isRunning(existing.status)
      const nowRunning = isRunning(heavy.status)

      if (wasRunning && !nowRunning) {
        slot.runningCount = Math.max(0, slot.runningCount - 1)
      } else if (!wasRunning && nowRunning) {
        slot.runningCount += 1
      }
      existing.status = heavy.status
    }

    if (heavy.kind !== undefined) {
      existing.kind = heavy.kind
    }

    if (heavy.identity !== undefined) {
      existing.identity = heavy.identity
    }

    if (Array.isArray(heavy.content)) {
      existing.content = heavy.content
    }

    if (heavy.rawInput !== undefined) {
      existing.rawInput = heavy.rawInput
    }

    if (Array.isArray(heavy.locations)) {
      existing.locations = heavy.locations
    }
    existing.formatted = heavy.formatted
    existing.startedAtMs = heavy.startedAtMs

    if (heavy.completedAtMs !== undefined) {
      existing.completedAtMs = heavy.completedAtMs
    }

    return
  }
  slot.calls.push({
    id: `tc-${toolCallId}`,
    agentId,
    sessionId,
    turnId: openTurnIdFor(id, sessionId),
    toolCallId,
    identity: heavy.identity,
    title: heavy.title,
    status: heavy.status,
    kind: heavy.kind,
    content: Array.isArray(heavy.content) ? heavy.content : [],
    rawInput: heavy.rawInput,
    locations: Array.isArray(heavy.locations) ? heavy.locations : undefined,
    formatted: heavy.formatted,
    startedAtMs: heavy.startedAtMs,
    completedAtMs: heavy.completedAtMs,
    createdAt: seq,
    updatedAt: seq
  })

  if (isRunning(heavy.status)) {
    slot.runningCount += 1
  }
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
  recountRunning(slot)

  return before - slot.calls.length
}

export function getToolCall(id: InstanceId, toolCallId: string): WireToolCall | undefined {
  return states.get(id)?.calls.find((c) => c.toolCallId === toolCallId)
}

export function useTools(instanceId?: InstanceId): { calls: ComputedRef<WireToolCall[]>; runningCount: ComputedRef<number> } {
  const { id: activeId } = useActiveInstance()
  const calls = computed<WireToolCall[]>(() => {
    const resolved = instanceId ?? activeId.value

    if (!resolved) {
      return []
    }

    return states.get(resolved)?.calls ?? []
  })
  const runningCount = computed<number>(() => {
    const resolved = instanceId ?? activeId.value

    if (!resolved) {
      return 0
    }

    return states.get(resolved)?.runningCount ?? 0
  })

  return { calls, runningCount }
}
