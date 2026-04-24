import { computed, reactive, type ComputedRef } from 'vue'

import { nextSeq } from './sequence'
import { useActiveInstance, type InstanceId } from './use-active-instance'

export enum StreamItemKind {
  Thought = 'thought',
  Plan = 'plan'
}

interface BaseStream {
  id: string
  sessionId: string
  createdAt: number
  updatedAt: number
}

export interface ThoughtStreamItem extends BaseStream {
  kind: StreamItemKind.Thought
  text: string
}

export interface PlanEntry {
  content?: string
  status?: string
  priority?: string
}

export interface PlanStreamItem extends BaseStream {
  kind: StreamItemKind.Plan
  entries: PlanEntry[]
}

export type StreamItem = ThoughtStreamItem | PlanStreamItem

export interface StreamState {
  items: StreamItem[]
}

const states = reactive(new Map<InstanceId, StreamState>())

function slotFor(id: InstanceId): StreamState {
  let slot = states.get(id)
  if (!slot) {
    slot = { items: [] }
    states.set(id, slot)
  }

  return slot
}

interface ThoughtUpdate {
  sessionUpdate: string
  content?: { text?: string }
  messageId?: string
}

interface PlanUpdate {
  sessionUpdate: string
  entries?: PlanEntry[]
}

export function pushThoughtChunk(id: InstanceId, sessionId: string, raw: ThoughtUpdate): void {
  const slot = slotFor(id)
  const seq = nextSeq(id)
  const text = typeof raw.content?.text === 'string' ? raw.content.text : ''
  const hasExplicitId = typeof raw.messageId === 'string'
  const last = slot.items[slot.items.length - 1]
  if (
    last
    && last.kind === StreamItemKind.Thought
    && last.sessionId === sessionId
    && (hasExplicitId ? last.id === raw.messageId : true)
  ) {
    last.text += text
    last.updatedAt = seq
    return
  }
  const itemId = hasExplicitId ? (raw.messageId as string) : `thought-${sessionId}-${slot.items.length}`
  slot.items.push({ kind: StreamItemKind.Thought, id: itemId, sessionId, createdAt: seq, updatedAt: seq, text })
}

export function pushPlan(id: InstanceId, sessionId: string, raw: PlanUpdate): void {
  const slot = slotFor(id)
  const seq = nextSeq(id)
  const entries = Array.isArray(raw.entries) ? raw.entries : []
  slot.items.push({
    kind: StreamItemKind.Plan,
    id: `plan-${sessionId}-${slot.items.length}`,
    sessionId,
    createdAt: seq,
    updatedAt: seq,
    entries
  })
}

export function resetStream(id: InstanceId): void {
  states.delete(id)
}

export function useStream(instanceId?: InstanceId): { items: ComputedRef<StreamItem[]> } {
  const { id: activeId } = useActiveInstance()
  const items = computed<StreamItem[]>(() => {
    const resolved = instanceId ?? activeId.value
    if (!resolved) {
      return []
    }

    return states.get(resolved)?.items ?? []
  })

  return { items }
}
