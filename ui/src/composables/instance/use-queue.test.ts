import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { Phase } from '@components'

import { useActiveInstance } from '@composables'
import { useAdapter } from '@composables'
import { __resetAllPhaseSignals, pushInstanceState, usePhase } from '@composables'
import {
  __resetAllQueues,
  flushQueue,
  popQueueHead,
  pushToQueue,
  resetQueue,
  startQueueDispatcher,
  stopQueueDispatcher,
  useQueue
} from '@composables'
import { clearToasts, useToasts } from '@composables'
import { resetTools } from '@composables'
import { __resetTurnEndedListeners, pushTurnEnded, pushTurnStarted, resetTurns } from '@composables'

import { InstanceState } from '@ipc'

const invoke = vi.fn()

vi.mock('@ipc', async () => {
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
    const a = pushToQueue('A', { text: 'first', pills: [], skillAttachments: [] })
    const b = pushToQueue('A', { text: 'second', pills: [], skillAttachments: [] })

    const { items } = useQueue('A')
    expect(items.value).toHaveLength(2)
    expect(items.value[0]?.id).toBe(a.id)
    expect(items.value[1]?.id).toBe(b.id)
    expect(a.enqueuedAt).toBeLessThan(b.enqueuedAt)
  })

  it('enqueue via composable resolves the active instance id', () => {
    useActiveInstance().set('A')
    const { enqueue, items } = useQueue()
    enqueue({ text: 'hi', pills: [], skillAttachments: [] })

    expect(items.value).toHaveLength(1)
    expect(items.value[0]?.text).toBe('hi')
  })

  it('popQueueHead returns FIFO and undefined when empty', () => {
    pushToQueue('A', { text: 'first', pills: [], skillAttachments: [] })
    pushToQueue('A', { text: 'second', pills: [], skillAttachments: [] })

    const head = popQueueHead('A')
    expect(head?.text).toBe('first')
    const head2 = popQueueHead('A')
    expect(head2?.text).toBe('second')
    expect(popQueueHead('A')).toBeUndefined()
  })

  it('flushQueue drops every item for the instance', () => {
    pushToQueue('A', { text: 'a', pills: [], skillAttachments: [] })
    pushToQueue('A', { text: 'b', pills: [], skillAttachments: [] })

    flushQueue('A')
    expect(useQueue('A').items.value).toHaveLength(0)
  })

  it('isolates queues between instances', () => {
    pushToQueue('A', { text: 'for A', pills: [], skillAttachments: [] })
    pushToQueue('B', { text: 'for B', pills: [], skillAttachments: [] })

    expect(useQueue('A').items.value.map((q) => q.text)).toEqual(['for A'])
    expect(useQueue('B').items.value.map((q) => q.text)).toEqual(['for B'])
  })

  it('resetQueue clears the slot', () => {
    pushToQueue('A', { text: 'x', pills: [], skillAttachments: [] })
    resetQueue('A')
    expect(useQueue('A').items.value).toHaveLength(0)
  })
})

describe('startQueueDispatcher', () => {
  it('dispatches the queue head on turn-ended with stopReason=end_turn', async () => {
    startQueueDispatcher()
    pushToQueue('A', { text: 'queued one', pills: [], skillAttachments: [] })
    pushToQueue('A', { text: 'queued two', pills: [], skillAttachments: [] })

    pushTurnStarted('A', { turnId: 't1', sessionId: 's-a' })
    pushTurnEnded('A', { turnId: 't1', sessionId: 's-a', stopReason: 'end_turn' })

    // microtask flush so the void submit() promise lands the mock call
    await Promise.resolve()
    await Promise.resolve()

    expect(invoke).toHaveBeenCalledTimes(1)
    const args = invoke.mock.calls[0]?.[1] as { text: string; instanceId: string }
    expect(args.text).toBe('queued one')
    expect(args.instanceId).toBe('A')
    // head popped; one queued item remains
    expect(useQueue('A').items.value.map((q) => q.text)).toEqual(['queued two'])
  })

  it('flushes the queue on stopReason=cancelled and emits a toast', () => {
    startQueueDispatcher()
    pushToQueue('A', { text: 'x', pills: [], skillAttachments: [] })
    pushToQueue('A', { text: 'y', pills: [], skillAttachments: [] })

    pushTurnStarted('A', { turnId: 't1', sessionId: 's-a' })
    pushTurnEnded('A', { turnId: 't1', sessionId: 's-a', stopReason: 'cancelled' })

    expect(useQueue('A').items.value).toHaveLength(0)
    expect(invoke).not.toHaveBeenCalled()
    const messages = useToasts().entries.value.map((t) => t.body)
    expect(messages).toContain('queue cleared')
  })

  it('does not toast on cancelled when the queue was already empty', () => {
    startQueueDispatcher()
    pushTurnStarted('A', { turnId: 't1', sessionId: 's-a' })
    pushTurnEnded('A', { turnId: 't1', sessionId: 's-a', stopReason: 'cancelled' })

    const messages = useToasts().entries.value.map((t) => t.body)
    expect(messages).not.toContain('queue cleared')
  })

  it('ignores other stop reasons (max_tokens / refusal) — head stays, queue stays', () => {
    startQueueDispatcher()
    pushToQueue('A', { text: 'still queued', pills: [], skillAttachments: [] })

    pushTurnStarted('A', { turnId: 't1', sessionId: 's-a' })
    pushTurnEnded('A', { turnId: 't1', sessionId: 's-a', stopReason: 'max_tokens' })
    pushTurnEnded('A', { turnId: 't1', sessionId: 's-a', stopReason: 'refusal' })

    expect(invoke).not.toHaveBeenCalled()
    expect(useQueue('A').items.value.map((q) => q.text)).toEqual(['still queued'])
  })

  it('isolates dispatch between instances', async () => {
    startQueueDispatcher()
    pushToQueue('A', { text: 'A item', pills: [], skillAttachments: [] })
    pushToQueue('B', { text: 'B item', pills: [], skillAttachments: [] })

    pushTurnStarted('A', { turnId: 't1', sessionId: 's-a' })
    pushTurnEnded('A', { turnId: 't1', sessionId: 's-a', stopReason: 'end_turn' })

    await Promise.resolve()
    await Promise.resolve()

    expect(invoke).toHaveBeenCalledTimes(1)
    const args = invoke.mock.calls[0]?.[1] as { text: string; instanceId: string }
    expect(args.instanceId).toBe('A')
    // B's queue is untouched
    expect(useQueue('B').items.value).toHaveLength(1)
  })

  it('stopQueueDispatcher unsubscribes — subsequent ends do nothing', async () => {
    startQueueDispatcher()
    stopQueueDispatcher()
    pushToQueue('A', { text: 'x', pills: [], skillAttachments: [] })

    pushTurnStarted('A', { turnId: 't1', sessionId: 's-a' })
    pushTurnEnded('A', { turnId: 't1', sessionId: 's-a', stopReason: 'end_turn' })

    await Promise.resolve()
    expect(invoke).not.toHaveBeenCalled()
    // queue intact
    expect(useQueue('A').items.value).toHaveLength(1)
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
      pushToQueue(instanceId, { text, pills: [], skillAttachments: [] })

      return { dispatched: false }
    }
    void useAdapter().submit({ text, instanceId })

    return { dispatched: true }
  }

  it('phase=Idle → submit dispatches through useAdapter', async () => {
    const r = routeSubmit('first message')
    expect(r.dispatched).toBe(true)

    await Promise.resolve()
    expect(invoke).toHaveBeenCalledTimes(1)
    expect(useQueue('A').items.value).toHaveLength(0)
  })

  it('phase=Working → submit enqueues, no invoke', () => {
    useActiveInstance().set('A')
    pushInstanceState('A', InstanceState.Running)
    pushTurnStarted('A', { turnId: 't-active', sessionId: 's-a' })
    expect(usePhase().phase.value).toBe(Phase.Working)

    const r = routeSubmit('second message')
    expect(r.dispatched).toBe(false)
    expect(invoke).not.toHaveBeenCalled()
    expect(useQueue('A').items.value.map((q) => q.text)).toEqual(['second message'])
  })
})
