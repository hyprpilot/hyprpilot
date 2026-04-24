import { computed, reactive, type ComputedRef } from 'vue'

import { nextSeq } from './sequence'
import { useActiveInstance, type InstanceId } from './useActiveInstance'

export interface ToolCallLocation {
  path?: string
  line?: number
}

export interface ToolCallContentBlock {
  type?: string
  text?: string
  [k: string]: unknown
}

export interface ToolCallView {
  id: string
  sessionId: string
  toolCallId: string
  title?: string
  status?: string
  kind?: string
  content: ToolCallContentBlock[]
  rawInput?: Record<string, unknown>
  locations?: ToolCallLocation[]
  createdAt: number
  updatedAt: number
}

export interface ToolsState {
  calls: ToolCallView[]
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
  content?: ToolCallContentBlock[]
  rawInput?: Record<string, unknown>
  locations?: ToolCallLocation[]
}

export function pushToolCall(id: InstanceId, sessionId: string, raw: ToolCallUpdate): void {
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
    return
  }
  slot.calls.push({
    id: `tc-${toolCallId}`,
    sessionId,
    toolCallId,
    title: raw.title,
    status: raw.status,
    kind: raw.kind,
    content: Array.isArray(raw.content) ? raw.content : [],
    rawInput: raw.rawInput,
    locations: Array.isArray(raw.locations) ? raw.locations : undefined,
    createdAt: seq,
    updatedAt: seq
  })
}

export function resetTools(id: InstanceId): void {
  states.delete(id)
}

export function useTools(instanceId?: InstanceId): { calls: ComputedRef<ToolCallView[]> } {
  const { id: activeId } = useActiveInstance()
  const calls = computed<ToolCallView[]>(() => {
    const resolved = instanceId ?? activeId.value
    if (!resolved) {
      return []
    }

    return states.get(resolved)?.calls ?? []
  })

  return { calls }
}
