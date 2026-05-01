import { computed, reactive, type ComputedRef } from 'vue'

import { nextSeq } from './sequence'
import { useActiveInstance, type InstanceId } from '../chrome/use-active-instance'
import { openTurnIdFor } from './use-turns'

export enum StreamItemKind {
  Thought = 'thought',
  Plan = 'plan',
  ModeChange = 'mode_change'
}

interface BaseStream {
  id: string
  sessionId: string
  /// Active turn id at receive time; `undefined` for spontaneous
  /// updates outside a turn. Consumers group by this when rendering.
  turnId?: string
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

/// Banner chip rendered between turns whenever the agent emits a
/// `current_mode_update` (claude-code switching from `plan` →
/// `default` after the user accepts the exit-plan permission, etc.).
/// `name` is the human label from `availableModes` when known;
/// `modeId` falls through when we don't have a name. `prevName` /
/// `prevModeId` are the values BEFORE this transition so the banner
/// can read `mode · plan → default` instead of just `mode → default`.
export interface ModeChangeStreamItem extends BaseStream {
  kind: StreamItemKind.ModeChange
  modeId: string
  name?: string
  prevModeId?: string
  prevName?: string
}

export type StreamItem = ThoughtStreamItem | PlanStreamItem | ModeChangeStreamItem

export interface StreamState {
  items: StreamItem[]
  /// Per-session id of the agent's open thought item for the current
  /// turn. Cleared on `user_message_chunk` (the next turn starting);
  /// every `agent_thought_chunk` in between appends to the same item.
  openThoughtBySession: Map<string, string>
  /// Per-session id of the open plan item for the current turn. Plans
  /// arrive as full snapshots, so subsequent updates replace `entries`
  /// in place rather than appending — but stay anchored to the same
  /// item id (same `createdAt`) until the turn closes.
  openPlanBySession: Map<string, string>
}

const states = reactive(new Map<InstanceId, StreamState>())

function slotFor(id: InstanceId): StreamState {
  let slot = states.get(id)
  if (!slot) {
    slot = { items: [], openThoughtBySession: new Map(), openPlanBySession: new Map() }
    states.set(id, slot)
  }

  return slot
}

/// Close the per-session turn — clears both thought and plan trackers.
/// Called from the demuxer when a `user_message_chunk` arrives, signalling
/// the previous agent turn is done and the next thought / plan should
/// open a fresh item.
export function closeTurn(id: InstanceId, sessionId: string): void {
  const slot = states.get(id)
  if (!slot) {
    return
  }
  slot.openThoughtBySession.delete(sessionId)
  slot.openPlanBySession.delete(sessionId)
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
  const explicitId = hasExplicitId ? (raw.messageId as string) : undefined
  const openId = explicitId ?? slot.openThoughtBySession.get(sessionId)

  if (openId) {
    const target = slot.items.find(
      (it): it is ThoughtStreamItem =>
        it.kind === StreamItemKind.Thought && it.sessionId === sessionId && it.id === openId
    )
    if (target) {
      target.text += text
      target.updatedAt = seq
      return
    }
  }

  const itemId = explicitId ?? `thought-${sessionId}-${slot.items.length}`
  slot.items.push({
    kind: StreamItemKind.Thought,
    id: itemId,
    sessionId,
    turnId: openTurnIdFor(id, sessionId),
    createdAt: seq,
    updatedAt: seq,
    text
  })
  slot.openThoughtBySession.set(sessionId, itemId)
}

export function pushPlan(id: InstanceId, sessionId: string, raw: PlanUpdate): void {
  const slot = slotFor(id)
  const seq = nextSeq(id)
  const entries = Array.isArray(raw.entries) ? raw.entries : []
  const openId = slot.openPlanBySession.get(sessionId)

  if (openId) {
    const target = slot.items.find(
      (it): it is PlanStreamItem => it.kind === StreamItemKind.Plan && it.sessionId === sessionId && it.id === openId
    )
    if (target) {
      target.entries = entries
      target.updatedAt = seq
      return
    }
  }

  const itemId = `plan-${sessionId}-${slot.items.length}`
  slot.items.push({
    kind: StreamItemKind.Plan,
    id: itemId,
    sessionId,
    turnId: openTurnIdFor(id, sessionId),
    createdAt: seq,
    updatedAt: seq,
    entries
  })
  slot.openPlanBySession.set(sessionId, itemId)
}

export interface ModeChangePush {
  modeId: string
  name?: string
  prevModeId?: string
  prevName?: string
}

/// Banner-only push: emits a fresh `ModeChangeStreamItem` for the
/// active turn (or no turn — solo block). De-dupes against the most
/// recent mode change so a noisy double-update from the agent doesn't
/// stack two identical banners.
export function pushModeChange(id: InstanceId, sessionId: string, change: ModeChangePush): void {
  const slot = slotFor(id)
  const seq = nextSeq(id)
  const last = slot.items[slot.items.length - 1]
  if (
    last &&
    last.kind === StreamItemKind.ModeChange &&
    last.sessionId === sessionId &&
    last.modeId === change.modeId
  ) {
    last.updatedAt = seq
    return
  }
  const itemId = `mode-${sessionId}-${slot.items.length}`
  slot.items.push({
    kind: StreamItemKind.ModeChange,
    id: itemId,
    sessionId,
    turnId: openTurnIdFor(id, sessionId),
    createdAt: seq,
    updatedAt: seq,
    modeId: change.modeId,
    name: change.name,
    prevModeId: change.prevModeId,
    prevName: change.prevName
  })
}

export function resetStream(id: InstanceId): void {
  states.delete(id)
}

/** Drop every stream item (thought / plan chunk) tagged with
 * `turnId`. Paired with `deleteTurnByTurnId` to fully remove a
 * cancelled / errored turn from the visible chat. */
export function deleteStreamByTurnId(id: InstanceId, turnId: string): number {
  const slot = states.get(id)
  if (!slot) {
    return 0
  }
  const before = slot.items.length
  slot.items = slot.items.filter((item) => item.turnId !== turnId)
  return before - slot.items.length
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
