import { beforeEach, describe, expect, it } from 'vitest'

import {
  __resetTurnEndedListeners,
  markThinkingEnd,
  markThinkingStart,
  onTurnEnded,
  openTurnIdFor,
  pushTurnEnded,
  pushTurnStarted,
  pushUsageUpdate,
  resetTurns,
  useActiveInstance,
  useTurns
} from '@composables'

beforeEach(() => {
  resetTurns('A')
  resetTurns('B')
  __resetTurnEndedListeners()
  useActiveInstance().id.value = undefined
})

describe('pushTurnStarted', () => {
  it('records a fresh turn record with sessionId + startedAtMs', () => {
    pushTurnStarted('A', { turnId: 't-1', sessionId: 's-1', startedAtMs: 1_000 })

    const turns = useTurns('A').turns.value

    expect(turns).toHaveLength(1)
    expect(turns[0]?.id).toBe('t-1')
    expect(turns[0]?.sessionId).toBe('s-1')
    expect(turns[0]?.startedAtMs).toBe(1_000)
    expect(turns[0]?.thinkingMs).toBe(0)
  })

  it('is idempotent on turn id — second push fills in timing without duplicating', () => {
    // Race ordering: a usage_update arrived first and synthesised a placeholder
    // (startedAtMs: 0). The eventual real turn-started fills in timing in place.
    pushUsageUpdate('A', 's-1', 't-1', { used: 1, size: 100 })
    pushTurnStarted('A', { turnId: 't-1', sessionId: 's-1', startedAtMs: 5_000 })

    const turns = useTurns('A').turns.value

    expect(turns).toHaveLength(1)
    expect(turns[0]?.id).toBe('t-1')
    expect(turns[0]?.startedAtMs).toBe(5_000)
    expect(turns[0]?.usage?.used).toBe(1)
  })

  it('drops stale orphan session entries (TurnEnded never landed)', () => {
    // Open turn on session A — TurnEnded gets dropped (broadcast lag,
    // daemon panic). Next TurnStarted is on session B; the orphan must
    // clear so phase doesn't pin to the dead turn.
    pushTurnStarted('A', { turnId: 't-old', sessionId: 's-old', startedAtMs: 1 })
    pushTurnStarted('A', { turnId: 't-new', sessionId: 's-new', startedAtMs: 2 })

    expect(openTurnIdFor('A', 's-old')).toBeUndefined()
    expect(openTurnIdFor('A', 's-new')).toBe('t-new')
  })
})

describe('pushUsageUpdate', () => {
  it('synthesises a placeholder turn record when turnId arrives ahead of TurnStarted', () => {
    // Live-event-before-turn-record race: remote authenticated mid-turn,
    // the matching turn-started fired before the WS subscriber was wired.
    pushUsageUpdate('A', 's-1', 't-future', { used: 12, size: 200 })

    const turns = useTurns('A').turns.value

    expect(turns).toHaveLength(1)
    expect(turns[0]?.id).toBe('t-future')
    expect(turns[0]?.startedAtMs).toBe(0) // sentinel for "no real timing yet"
    expect(turns[0]?.usage).toEqual({ used: 12, size: 200 })
  })

  it('binds to the open turn for the session when no turnId is supplied', () => {
    pushTurnStarted('A', { turnId: 't-1', sessionId: 's-1', startedAtMs: 1 })
    pushUsageUpdate('A', 's-1', undefined, { used: 5, size: 100 })

    expect(useTurns('A').turns.value[0]?.usage).toEqual({ used: 5, size: 100 })
  })

  it('drops the reading when no matching turn exists and no turnId is supplied', () => {
    // Between-turn usage update with no anchor — nothing to bind to.
    pushUsageUpdate('A', 's-1', undefined, { used: 5, size: 100 })

    expect(useTurns('A').turns.value).toHaveLength(0)
  })
})

describe('markThinkingStart / markThinkingEnd', () => {
  it('is a no-op when no turn is open for the session', () => {
    // No pushTurnStarted yet — markThinkingStart short-circuits.
    markThinkingStart('A', 's-orphan', 1_000)
    markThinkingEnd('A', 's-orphan', 2_000)

    expect(useTurns('A').turns.value).toHaveLength(0)
  })

  it('accumulates thinking intervals across the same turn', () => {
    pushTurnStarted('A', { turnId: 't-1', sessionId: 's-1', startedAtMs: 0 })

    markThinkingStart('A', 's-1', 100)
    markThinkingEnd('A', 's-1', 250) // +150ms
    markThinkingStart('A', 's-1', 400)
    markThinkingEnd('A', 's-1', 700) // +300ms

    expect(useTurns('A').turns.value[0]?.thinkingMs).toBe(450)
  })

  it('markThinkingStart is idempotent — second call while open does not reopen', () => {
    pushTurnStarted('A', { turnId: 't-1', sessionId: 's-1', startedAtMs: 0 })
    markThinkingStart('A', 's-1', 100)
    markThinkingStart('A', 's-1', 200) // ignored — interval already open
    markThinkingEnd('A', 's-1', 300)

    expect(useTurns('A').turns.value[0]?.thinkingMs).toBe(200) // 300 - 100, not 100
  })
})

describe('pushTurnEnded', () => {
  it('clears the open-session pointer + fires registered listeners', () => {
    pushTurnStarted('A', { turnId: 't-1', sessionId: 's-1', startedAtMs: 0 })

    const fired: string[] = []

    onTurnEnded((id, raw) => {
      fired.push(`${id}/${raw.turnId}/${raw.stopReason}`)
    })

    pushTurnEnded('A', { turnId: 't-1', sessionId: 's-1', endedAtMs: 1_000, stopReason: 'end_turn' })

    expect(openTurnIdFor('A', 's-1')).toBeUndefined()
    expect(fired).toEqual(['A/t-1/end_turn'])
    expect(useTurns('A').turns.value[0]?.endedAtMs).toBe(1_000)
    expect(useTurns('A').turns.value[0]?.stopReason).toBe('end_turn')
  })

  it('closes a still-open thinking interval', () => {
    pushTurnStarted('A', { turnId: 't-1', sessionId: 's-1', startedAtMs: 0 })
    markThinkingStart('A', 's-1', 100)
    pushTurnEnded('A', { turnId: 't-1', sessionId: 's-1', endedAtMs: 600, stopReason: 'end_turn' })

    expect(useTurns('A').turns.value[0]?.thinkingMs).toBe(500)
    expect(useTurns('A').turns.value[0]?.thinkingOpenAtMs).toBeUndefined()
  })
})
