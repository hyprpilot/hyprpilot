import { computed, reactive, type ComputedRef } from 'vue'

import { nextSeq } from './sequence'
import { useActiveInstance, type InstanceId } from './use-active-instance'

export interface TurnRecord {
  id: string
  instanceId: InstanceId
  sessionId: string
  /// Monotonic sequence captured at TurnStarted dispatch — drives
  /// timeline ordering relative to other entries (turns, tool calls,
  /// stream items) so a turn always sorts after the user prompt that
  /// opened it and before the first chunk landing inside it.
  createdAt: number
  /// Set when `acp:turn-ended` arrives; used by the live-block
  /// computation to decide which turn is still streaming.
  endedAt?: number
  /// ACP `StopReason` wire string when the turn ended cleanly.
  stopReason?: string
}

interface TurnsState {
  turns: TurnRecord[]
  /// Per-session pointer at the currently-open turn id. Cleared when
  /// the matching `acp:turn-ended` lands; consulted by the
  /// stream/transcript/tool stores to stamp the active turn id onto
  /// each pushed entry.
  openBySession: Map<string, string>
}

const states = reactive(new Map<InstanceId, TurnsState>())

function slotFor(id: InstanceId): TurnsState {
  let slot = states.get(id)
  if (!slot) {
    slot = { turns: [], openBySession: new Map() }
    states.set(id, slot)
  }

  return slot
}

export interface TurnStartedRaw {
  turnId: string
  sessionId: string
}

export interface TurnEndedRaw {
  turnId: string
  sessionId: string
  stopReason?: string
}

export function pushTurnStarted(id: InstanceId, raw: TurnStartedRaw): void {
  const slot = slotFor(id)
  const seq = nextSeq(id)
  slot.turns.push({
    id: raw.turnId,
    instanceId: id,
    sessionId: raw.sessionId,
    createdAt: seq
  })
  slot.openBySession.set(raw.sessionId, raw.turnId)
}

export type TurnEndedListener = (id: InstanceId, raw: TurnEndedRaw) => void

const turnEndedListeners = new Set<TurnEndedListener>()

/**
 * Register a sibling-store hook that fires after `pushTurnEnded`
 * lands its mutation. Returns the unsubscribe fn. Used by
 * `use-queue.ts` to dispatch the queue head on `end_turn` and
 * cancel-flush on `cancelled`.
 */
export function onTurnEnded(listener: TurnEndedListener): () => void {
  turnEndedListeners.add(listener)
  return () => {
    turnEndedListeners.delete(listener)
  }
}

export function __resetTurnEndedListeners(): void {
  turnEndedListeners.clear()
}

export function pushTurnEnded(id: InstanceId, raw: TurnEndedRaw): void {
  const slot = slotFor(id)
  const seq = nextSeq(id)
  const target = slot.turns.find((t) => t.id === raw.turnId)
  if (target) {
    target.endedAt = seq
    target.stopReason = raw.stopReason
  }
  if (slot.openBySession.get(raw.sessionId) === raw.turnId) {
    slot.openBySession.delete(raw.sessionId)
  }
  for (const listener of turnEndedListeners) {
    listener(id, raw)
  }
}

export function resetTurns(id: InstanceId): void {
  states.delete(id)
}

/// Read the open turn id for a given session — used by sibling stores
/// to stamp `turnId` onto pushed items at receive time. Returns
/// `undefined` when no turn is currently in flight for that session
/// (out-of-turn agent updates fall through with no turn anchor).
export function openTurnIdFor(id: InstanceId, sessionId: string): string | undefined {
  return states.get(id)?.openBySession.get(sessionId)
}

export function useTurns(instanceId?: InstanceId): {
  turns: ComputedRef<TurnRecord[]>
  openTurnId: ComputedRef<string | undefined>
} {
  const { id: activeId } = useActiveInstance()
  const turns = computed<TurnRecord[]>(() => {
    const resolved = instanceId ?? activeId.value
    if (!resolved) {
      return []
    }

    return states.get(resolved)?.turns ?? []
  })
  const openTurnId = computed<string | undefined>(() => {
    const resolved = instanceId ?? activeId.value
    if (!resolved) {
      return undefined
    }
    const slot = states.get(resolved)
    if (!slot) {
      return undefined
    }
    // Multi-session per instance is theoretical today (one ACP session
    // per actor), so picking any open id is correct; future work might
    // index by sessionId from caller context.
    const it = slot.openBySession.values().next()

    return it.done ? undefined : it.value
  })

  return { turns, openTurnId }
}
