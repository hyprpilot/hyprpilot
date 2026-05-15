import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import {
  __resetAllQueues,
  applyQueueChanged,
  refreshQueue,
  resetQueue,
  useActiveInstance,
  useQueue
} from '@composables'
import type { QueueItem } from '@interfaces/wire/queue'
import { TauriCommand } from '@ipc'

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }))

vi.mock('@ipc', async() => ({
  ...(await vi.importActual<object>('@ipc')),
  invoke: (cmd: string, args?: Record<string, unknown>) => invoke(cmd, args),
  listen: () => Promise.resolve(() => {})
}))

const flushMicrotasks = (): Promise<void> => new Promise((r) => setTimeout(r, 0))

function item(id: string, seq: number, text = 'hi'): QueueItem {
  return {
    id,
    text,
    enqueuedSeq: seq,
    enqueuedAt: seq
  }
}

beforeEach(() => {
  invoke.mockReset()
  invoke.mockResolvedValue({ items: [] })
  __resetAllQueues()
  useActiveInstance().id.value = undefined
})

afterEach(() => {
  __resetAllQueues()
})

describe('useQueue (daemon-mirror)', () => {
  it('applyQueueChanged replaces wholesale — no merge, no carry-over', () => {
    applyQueueChanged('A', [item('q-1', 1), item('q-2', 2)])
    expect(useQueue('A').items.value.map((q) => q.id)).toEqual(['q-1', 'q-2'])

    applyQueueChanged('A', [item('q-3', 3)])
    expect(useQueue('A').items.value.map((q) => q.id)).toEqual(['q-3'])
  })

  it('applyQueueChanged accepts an empty list (clear-via-empty contract)', () => {
    applyQueueChanged('A', [item('q-1', 1)])
    applyQueueChanged('A', [])
    expect(useQueue('A').items.value).toHaveLength(0)
  })

  it('isolates instance slices: A never sees B and vice versa', () => {
    applyQueueChanged('A', [item('q-a', 1)])
    applyQueueChanged('B', [item('q-b', 1)])
    expect(useQueue('A').items.value.map((q) => q.id)).toEqual(['q-a'])
    expect(useQueue('B').items.value.map((q) => q.id)).toEqual(['q-b'])
  })

  it('useQueue(undefined) resolves through the active instance id', () => {
    useActiveInstance().set('A')
    applyQueueChanged('A', [item('q-1', 1)])
    expect(useQueue().items.value.map((q) => q.id)).toEqual(['q-1'])
  })

  it('first observation of an unseen id fires exactly one snapshot invoke', async() => {
    invoke.mockResolvedValueOnce({ items: [item('q-fetched', 1)] })

    const a1 = useQueue('A')

    // Force the watchEffect to run.
    await flushMicrotasks()
    expect(invoke).toHaveBeenCalledTimes(1)
    expect(invoke).toHaveBeenCalledWith(TauriCommand.InstanceSnapshotQueue, { instanceId: 'A' })

    // Second observer for the same instance must NOT re-fire — the
    // observed-set gates concurrent first-observations.
    const a2 = useQueue('A')

    await flushMicrotasks()
    expect(invoke).toHaveBeenCalledTimes(1)
    expect(a1.items.value).toHaveLength(1)
    expect(a2.items.value).toHaveLength(1)
  })

  it('refreshQueue swallows invoke errors and leaves the store untouched', async() => {
    applyQueueChanged('A', [item('q-1', 1)])
    invoke.mockRejectedValueOnce(new Error('daemon down'))

    await refreshQueue('A')
    expect(useQueue('A').items.value.map((q) => q.id)).toEqual(['q-1'])
  })

  it('drops a stale snapshot whose newest seq is older than what we already observed', async() => {
    applyQueueChanged('A', [item('q-fresh', 10)])

    // Daemon snapshot reply with stale data (seq 5 < 10).
    invoke.mockResolvedValueOnce({ items: [item('q-stale', 5)] })
    await refreshQueue('A')

    // Watermark guard kept the fresh broadcast state.
    expect(useQueue('A').items.value.map((q) => q.id)).toEqual(['q-fresh'])
  })

  it('accepts an empty snapshot even when the watermark is high (clear-via-empty)', async() => {
    applyQueueChanged('A', [item('q-1', 10)])
    invoke.mockResolvedValueOnce({ items: [] })

    await refreshQueue('A')
    expect(useQueue('A').items.value).toHaveLength(0)
  })

  it('resetQueue clears the slot AND the observed marker', async() => {
    // First useQueue('A') marks A as observed via the watchEffect.
    useQueue('A')
    await flushMicrotasks()
    expect(invoke).toHaveBeenCalledTimes(1)

    // Reset wipes both the cached items AND the observed marker.
    applyQueueChanged('A', [item('q-1', 1)])
    resetQueue('A')
    expect(useQueue('A').items.value).toHaveLength(0)

    // Post-reset, the NEXT useQueue('A') is a fresh first observation
    // → re-fires the snapshot invoke. (Note: the immediately-prior
    // `useQueue('A').items.value` re-marks A as observed via its
    // watchEffect, so we have to test the contract by calling
    // resetQueue AGAIN before the final invocation.)
    resetQueue('A')
    invoke.mockClear()
    invoke.mockResolvedValueOnce({ items: [item('q-fetched', 1)] })
    useQueue('A')
    await flushMicrotasks()
    expect(invoke).toHaveBeenCalledTimes(1)
  })

  it('mutation API methods route through invoke with the right command + args', async() => {
    useActiveInstance().set('A')
    const q = useQueue()

    invoke.mockResolvedValue({ item: item('q-1', 1, 'edited') })
    await q.edit('q-1', 'edited')
    expect(invoke).toHaveBeenCalledWith(
      TauriCommand.QueueEdit,
      expect.objectContaining({
        instanceId: 'A',
        itemId: 'q-1',
        text: 'edited'
      })
    )

    invoke.mockResolvedValue({ removed: true })
    await q.remove('q-1')
    expect(invoke).toHaveBeenCalledWith(TauriCommand.QueueRemove, { instanceId: 'A', itemId: 'q-1' })

    invoke.mockResolvedValue({
      accepted: true,
      item: item('q-2', 2)
    })
    await q.dispatch('q-2')
    expect(invoke).toHaveBeenCalledWith(TauriCommand.QueueDispatch, { instanceId: 'A', itemId: 'q-2' })

    invoke.mockResolvedValue({ moved: true })
    await q.move('q-1', 3)
    expect(invoke).toHaveBeenCalledWith(TauriCommand.QueueMove, {
      instanceId: 'A',
      itemId: 'q-1',
      position: 3
    })

    invoke.mockResolvedValue({ cleared: 4 })
    await q.clear()
    expect(invoke).toHaveBeenCalledWith(TauriCommand.QueueClear, { instanceId: 'A' })
  })
})
