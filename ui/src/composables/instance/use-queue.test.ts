import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { Phase } from '@components'
import {
  useActiveInstance,
  useAdapter,
  __resetAllPhaseSignals,
  pushInstanceState,
  usePhase,
  __resetAllQueues,
  dispatchQueueHead,
  dispatchQueueItem,
  flushQueue,
  popQueueHead,
  popQueueItem,
  pushToQueue,
  pushToQueueAt,
  resetQueue,
  startQueueDispatcher,
  stopQueueDispatcher,
  useQueue,
  clearToasts,
  resetTools,
  __resetTurnEndedListeners,
  pushTurnEnded,
  pushTurnStarted,
  resetTurns
} from '@composables'
import { InstanceState } from '@ipc'

const invoke = vi.fn()

vi.mock('@ipc', async() => {
  const actual = await vi.importActual<typeof import('@ipc')>('@ipc')

  return {
    ...actual,
    invoke: (command: string, args?: Record<string, unknown>) => invoke(command, args),
    listen: vi.fn()
  }
})

beforeEach(() => {
  invoke.mockReset()
  invoke.mockResolvedValue({ accepted: true })
  __resetAllQueues()
  __resetTurnEndedListeners()
  __resetAllPhaseSignals()
  resetTurns('A')
  resetTurns('B')
  resetTools('A')
  resetTools('B')
  clearToasts()
  useActiveInstance().id.value = undefined
})

afterEach(() => {
  stopQueueDispatcher()
})

describe('useQueue', () => {
  it('enqueue appends to the tail and stamps id + enqueuedAt', () => {
    const a = pushToQueue('A', {
      text: 'first',
      pills: [],
      skillAttachments: []
    })
    const b = pushToQueue('A', {
      text: 'second',
      pills: [],
      skillAttachments: []
    })

    const { items } = useQueue('A')

    expect(items.value).toHaveLength(2)
    expect(items.value[0]?.id).toBe(a.id)
    expect(items.value[1]?.id).toBe(b.id)
    expect(a.enqueuedAt).toBeLessThan(b.enqueuedAt)
  })

  it('enqueue via composable resolves the active instance id', () => {
    useActiveInstance().set('A')
    const { enqueue, items } = useQueue()

    enqueue({
      text: 'hi',
      pills: [],
      skillAttachments: []
    })

    expect(items.value).toHaveLength(1)
    expect(items.value[0]?.text).toBe('hi')
  })

  it('popQueueHead returns FIFO and undefined when empty', () => {
    pushToQueue('A', {
      text: 'first',
      pills: [],
      skillAttachments: []
    })
    pushToQueue('A', {
      text: 'second',
      pills: [],
      skillAttachments: []
    })

    const head = popQueueHead('A')

    expect(head?.text).toBe('first')
    const head2 = popQueueHead('A')

    expect(head2?.text).toBe('second')
    expect(popQueueHead('A')).toBeUndefined()
  })

  it('flushQueue drops every item for the instance', () => {
    pushToQueue('A', {
      text: 'a',
      pills: [],
      skillAttachments: []
    })
    pushToQueue('A', {
      text: 'b',
      pills: [],
      skillAttachments: []
    })

    flushQueue('A')
    expect(useQueue('A').items.value).toHaveLength(0)
  })

  it('isolates queues between instances', () => {
    pushToQueue('A', {
      text: 'for A',
      pills: [],
      skillAttachments: []
    })
    pushToQueue('B', {
      text: 'for B',
      pills: [],
      skillAttachments: []
    })

    expect(useQueue('A').items.value.map((q) => q.text)).toEqual(['for A'])
    expect(useQueue('B').items.value.map((q) => q.text)).toEqual(['for B'])
  })

  it('resetQueue clears the slot', () => {
    pushToQueue('A', {
      text: 'x',
      pills: [],
      skillAttachments: []
    })
    resetQueue('A')
    expect(useQueue('A').items.value).toHaveLength(0)
  })
})

describe('popQueueItem + pushToQueueAt (edit round-trip)', () => {
  it('popQueueItem removes the entry and reports its slot', () => {
    const a = pushToQueue('A', {
      text: 'first',
      pills: [],
      skillAttachments: []
    })
    const b = pushToQueue('A', {
      text: 'second',
      pills: [],
      skillAttachments: []
    })
    const c = pushToQueue('A', {
      text: 'third',
      pills: [],
      skillAttachments: []
    })

    const popped = popQueueItem('A', b.id)

    expect(popped?.position).toBe(1)
    expect(popped?.item.text).toBe('second')
    expect(useQueue('A').items.value.map((q) => q.id)).toEqual([a.id, c.id])
  })

  it('popQueueItem returns undefined for an unknown id', () => {
    pushToQueue('A', {
      text: 'only',
      pills: [],
      skillAttachments: []
    })
    expect(popQueueItem('A', 'no-such-id')).toBeUndefined()
  })

  it('pushToQueueAt inserts at the given slot, clamped to bounds', () => {
    pushToQueue('A', {
      text: 'first',
      pills: [],
      skillAttachments: []
    })
    pushToQueue('A', {
      text: 'third',
      pills: [],
      skillAttachments: []
    })

    pushToQueueAt('A', 1, {
      text: 'second',
      pills: [],
      skillAttachments: []
    })
    expect(useQueue('A').items.value.map((q) => q.text)).toEqual(['first', 'second', 'third'])

    pushToQueueAt('A', 999, {
      text: 'tail',
      pills: [],
      skillAttachments: []
    })
    expect(useQueue('A').items.value.map((q) => q.text)).toEqual(['first', 'second', 'third', 'tail'])
  })
})

describe('dispatchQueueHead / dispatchQueueItem', () => {
  it('dispatchQueueHead pops the head and submits via the adapter', async() => {
    pushToQueue('A', {
      text: 'queued one',
      pills: [],
      skillAttachments: []
    })
    pushToQueue('A', {
      text: 'queued two',
      pills: [],
      skillAttachments: []
    })

    dispatchQueueHead('A')

    await Promise.resolve()
    await Promise.resolve()

    expect(invoke).toHaveBeenCalledTimes(1)
    const args = invoke.mock.calls[0]?.[1] as { text: string; instanceId: string }

    expect(args.text).toBe('queued one')
    expect(useQueue('A').items.value.map((q) => q.text)).toEqual(['queued two'])
  })

  it('dispatchQueueHead is a no-op when the queue is empty', () => {
    dispatchQueueHead('A')
    expect(invoke).not.toHaveBeenCalled()
  })

  it('dispatchQueueItem pops a specific entry and submits it', async() => {
    pushToQueue('A', {
      text: 'a',
      pills: [],
      skillAttachments: []
    })
    const b = pushToQueue('A', {
      text: 'b',
      pills: [],
      skillAttachments: []
    })

    dispatchQueueItem('A', b.id)

    await Promise.resolve()
    await Promise.resolve()

    expect(invoke).toHaveBeenCalledTimes(1)
    const args = invoke.mock.calls[0]?.[1] as { text: string; instanceId: string }

    expect(args.text).toBe('b')
    expect(useQueue('A').items.value.map((q) => q.text)).toEqual(['a'])
  })
})

describe('startQueueDispatcher', () => {
  it('does NOT auto-dispatch the queue head on turn-ended end_turn — captain only', async() => {
    startQueueDispatcher()
    pushToQueue('A', {
      text: 'queued one',
      pills: [],
      skillAttachments: []
    })

    pushTurnStarted('A', {
      turnId: 't1', sessionId: 's-a', startedAtMs: 0
    })
    pushTurnEnded('A', {
      turnId: 't1',
      sessionId: 's-a',
      stopReason: 'end_turn', endedAtMs: 0
    })

    await Promise.resolve()
    await Promise.resolve()

    expect(invoke).not.toHaveBeenCalled()
    // head still queued — captain drains via Ctrl+Enter or the strip
    expect(useQueue('A').items.value.map((q) => q.text)).toEqual(['queued one'])
  })

  it('leaves the queue intact on stopReason=cancelled — captain only drains via the strip', () => {
    // Cancel-flush coupling was dropped: Ctrl+C cancels the
    // in-flight turn but queued items survive so the captain can
    // let them dispatch on the next turn.
    startQueueDispatcher()
    pushToQueue('A', {
      text: 'x',
      pills: [],
      skillAttachments: []
    })
    pushToQueue('A', {
      text: 'y',
      pills: [],
      skillAttachments: []
    })

    pushTurnStarted('A', {
      turnId: 't1', sessionId: 's-a', startedAtMs: 0
    })
    pushTurnEnded('A', {
      turnId: 't1',
      sessionId: 's-a',
      stopReason: 'cancelled', endedAtMs: 0
    })

    expect(useQueue('A').items.value).toHaveLength(2)
    expect(invoke).not.toHaveBeenCalled()
  })

  it('ignores other stop reasons (max_tokens / refusal) — head stays, queue stays', () => {
    startQueueDispatcher()
    pushToQueue('A', {
      text: 'still queued',
      pills: [],
      skillAttachments: []
    })

    pushTurnStarted('A', {
      turnId: 't1', sessionId: 's-a', startedAtMs: 0
    })
    pushTurnEnded('A', {
      turnId: 't1',
      sessionId: 's-a',
      stopReason: 'max_tokens', endedAtMs: 0
    })
    pushTurnEnded('A', {
      turnId: 't1',
      sessionId: 's-a',
      stopReason: 'refusal', endedAtMs: 0
    })

    expect(invoke).not.toHaveBeenCalled()
    expect(useQueue('A').items.value.map((q) => q.text)).toEqual(['still queued'])
  })

  it('keeps sibling queues intact regardless of stop reason', () => {
    startQueueDispatcher()
    pushToQueue('A', {
      text: 'A item',
      pills: [],
      skillAttachments: []
    })
    pushToQueue('B', {
      text: 'B item',
      pills: [],
      skillAttachments: []
    })

    pushTurnStarted('A', {
      turnId: 't1', sessionId: 's-a', startedAtMs: 0
    })
    pushTurnEnded('A', {
      turnId: 't1',
      sessionId: 's-a',
      stopReason: 'cancelled', endedAtMs: 0
    })

    expect(useQueue('A').items.value).toHaveLength(1)
    expect(useQueue('B').items.value).toHaveLength(1)
  })
})

/**
 * Mirrors the `Overlay.vue::onSubmit` routing decision: when phase is
 * idle, dispatch via `useAdapter().submit`; otherwise enqueue. Lives
 * here (not in `Chat.test.ts`) so the existing baseline harness
 * issues don't bleed into the queue invariant.
 */
describe('submit-routing (Overlay.vue parity)', () => {
  function routeSubmit(text: string): { dispatched: boolean } {
    useActiveInstance().setIfUnset('A')
    const instanceId = useActiveInstance().id.value!
    const { phase } = usePhase()

    if (phase.value !== Phase.Idle) {
      pushToQueue(instanceId, {
        text,
        pills: [],
        skillAttachments: []
      })

      return { dispatched: false }
    }
    void useAdapter().submit({ text, instanceId })

    return { dispatched: true }
  }

  it('phase=Idle → submit dispatches through useAdapter', async() => {
    const r = routeSubmit('first message')

    expect(r.dispatched).toBe(true)

    await Promise.resolve()
    expect(invoke).toHaveBeenCalledTimes(1)
    expect(useQueue('A').items.value).toHaveLength(0)
  })

  it('phase=Working → submit enqueues, no invoke', () => {
    useActiveInstance().set('A')
    pushInstanceState('A', InstanceState.Running)
    pushTurnStarted('A', {
      turnId: 't-active', sessionId: 's-a', startedAtMs: 0
    })
    expect(usePhase().phase.value).toBe(Phase.Working)

    const r = routeSubmit('second message')

    expect(r.dispatched).toBe(false)
    expect(invoke).not.toHaveBeenCalled()
    expect(useQueue('A').items.value.map((q) => q.text)).toEqual(['second message'])
  })
})
