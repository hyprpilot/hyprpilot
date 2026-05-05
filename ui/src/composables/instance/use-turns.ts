import { computed, reactive, type ComputedRef } from 'vue'

import { nextSeq } from './sequence'
import { useActiveInstance, type InstanceId } from '../chrome/use-active-instance'

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
  /// Wall-clock (epoch ms) — daemon-stamped on TurnStarted /
  /// TurnEnded so the UI can render a total-elapsed footer chip.
  /// `endedAtMs` undefined while the turn is in flight.
  startedAtMs: number
  endedAtMs?: number
  /// Accumulated thinking time across this turn's reasoning phases
  /// (every `agent_thought_chunk` interval, ended by the next
  /// `agent_message_chunk` / `tool_call` / `TurnEnded`). The agent
  /// can think → write → think again multiple times in a single
  /// turn; this sums every closed interval. Live tick adds
  /// `(now - thinkingOpenAtMs)` when an interval is currently open.
  thinkingMs: number
  /// Wall-clock when the current open thinking interval began;
  /// `undefined` while the agent is writing / executing tools.
  thinkingOpenAtMs?: number
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
  startedAtMs: number
}

export interface TurnEndedRaw {
  turnId: string
  sessionId: string
  stopReason?: string
  endedAtMs: number
}

// ── Internal store-mutation surface ───────────────────────────────
// The exports below sit outside the composable's `useTurns()` API
// surface on purpose: they're the wire-listener-facing inputs that
// sibling stores (use-stream, use-session-stream) push raw event
// payloads through. Per-feature views consume `useTurns()`; the wire
// router / sibling stores use these free fns directly.
// See CLAUDE.md ▸ "Two-tier composables: store API vs sibling-store
// mutation surface" for the convention.

export function pushTurnStarted(id: InstanceId, raw: TurnStartedRaw): void {
  const slot = slotFor(id)
  const seq = nextSeq(id)

  slot.turns.push({
    id: raw.turnId,
    instanceId: id,
    sessionId: raw.sessionId,
    createdAt: seq,
    startedAtMs: raw.startedAtMs,
    thinkingMs: 0
  })
  slot.openBySession.set(raw.sessionId, raw.turnId)
}

/// Mark the agent as entering its reasoning phase. Idempotent — a
/// second `agent_thought_chunk` while already thinking is just a
/// continuation of the open interval. Looks up the live turn for
/// `sessionId` so the caller doesn't need a turn id.
export function markThinkingStart(id: InstanceId, sessionId: string, atMs: number = Date.now()): void {
  const slot = states.get(id)

  if (!slot) {
    return
  }
  const turnId = slot.openBySession.get(sessionId)

  if (!turnId) {
    return
  }
  const turn = slot.turns.find((t) => t.id === turnId)

  if (!turn || turn.thinkingOpenAtMs !== undefined) {
    return
  }
  turn.thinkingOpenAtMs = atMs
}

/// Close the open thinking interval — agent moved to writing /
/// tool execution / turn end. Idempotent when no interval is open.
export function markThinkingEnd(id: InstanceId, sessionId: string, atMs: number = Date.now()): void {
  const slot = states.get(id)

  if (!slot) {
    return
  }
  const turnId = slot.openBySession.get(sessionId)

  if (!turnId) {
    return
  }
  const turn = slot.turns.find((t) => t.id === turnId)

  if (!turn || turn.thinkingOpenAtMs === undefined) {
    return
  }
  turn.thinkingMs += Math.max(0, atMs - turn.thinkingOpenAtMs)
  turn.thinkingOpenAtMs = undefined
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
    target.endedAtMs = raw.endedAtMs

    if (target.thinkingOpenAtMs !== undefined) {
      target.thinkingMs += Math.max(0, raw.endedAtMs - target.thinkingOpenAtMs)
      target.thinkingOpenAtMs = undefined
    }
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
