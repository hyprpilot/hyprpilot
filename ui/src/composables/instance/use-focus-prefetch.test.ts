import { QueryClient } from '@tanstack/vue-query'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { brimSync, prefetchInstanceMeta, prefetchInstanceChatFirstPage, useFocusPrefetch } from './use-focus-prefetch'
import { __resetActiveInstanceForTests, useActiveInstance } from '../chrome/use-active-instance'
import { TauriCommand, TauriEvent } from '@ipc'

const { invoke, listeners } = vi.hoisted(() => ({
  invoke: vi.fn(),
  listeners: new Map<string, (payload: { payload: unknown }) => void>()
}))

vi.mock('@ipc', async() => ({
  ...(await vi.importActual<object>('@ipc')),
  invoke: (command: string, args?: Record<string, unknown>) => invoke(command, args),
  listen: (event: string, cb: (payload: { payload: unknown }) => void) => {
    listeners.set(event, cb)

    return Promise.resolve(() => listeners.delete(event))
  }
}))

function buildClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: {
        retry: false,
        gcTime: 60_000,
        staleTime: 0
      }
    }
  })
}

beforeEach(() => {
  invoke.mockReset()
  listeners.clear()
  __resetActiveInstanceForTests()
})

describe('prefetchInstanceMeta', () => {
  it('invokes instance_snapshot_meta and seeds the cache', async() => {
    invoke.mockResolvedValue({ mcpsCount: 2, usage: { used: 0, size: 0 } })
    const client = buildClient()

    await prefetchInstanceMeta(client, 'i-1')

    expect(invoke).toHaveBeenCalledWith(TauriCommand.InstanceSnapshotMeta, { instanceId: 'i-1' })

    const cached = client.getQueryData(['snapshot-meta', 'i-1']) as { mcpsCount: number } | undefined

    expect(cached?.mcpsCount).toBe(2)
  })
})

describe('prefetchInstanceChatFirstPage', () => {
  it('invokes instance_snapshot_chat with no `before` cursor', async() => {
    invoke.mockResolvedValue({ items: [], hasMore: false })
    const client = buildClient()

    await prefetchInstanceChatFirstPage(client, 'i-1')

    expect(invoke).toHaveBeenCalledWith(TauriCommand.InstanceSnapshotChat, expect.objectContaining({ instanceId: 'i-1', before: undefined }))
  })
})

describe('brimSync', () => {
  it('lists instances then prefetches meta for each + chat for the focused id', async() => {
    invoke.mockImplementation((command: string) => {
      if (command === TauriCommand.InstancesList) {
        return Promise.resolve({
          instances: [
            {
              agentId: 'a',
              instanceId: 'i-1',
              name: undefined
            },
            {
              agentId: 'a',
              instanceId: 'i-2',
              name: undefined
            },
            {
              agentId: 'a',
              instanceId: 'i-3',
              name: undefined
            }
          ]
        })
      }

      if (command === TauriCommand.InstanceSnapshotMeta) {
        return Promise.resolve({ mcpsCount: 0, usage: { used: 0, size: 0 } })
      }

      if (command === TauriCommand.InstanceSnapshotChat) {
        return Promise.resolve({ items: [], hasMore: false })
      }

      return Promise.reject(new Error(`unexpected command ${command}`))
    })
    const client = buildClient()

    await brimSync(client, 'i-2')

    const calls = invoke.mock.calls.map((c) => c[0])

    expect(calls.filter((c) => c === TauriCommand.InstancesList)).toHaveLength(1)
    expect(calls.filter((c) => c === TauriCommand.InstanceSnapshotMeta)).toHaveLength(3)
    expect(calls.filter((c) => c === TauriCommand.InstanceSnapshotChat)).toHaveLength(1)

    // Meta cache populated for every instance.
    expect(client.getQueryData(['snapshot-meta', 'i-1'])).toBeDefined()
    expect(client.getQueryData(['snapshot-meta', 'i-2'])).toBeDefined()
    expect(client.getQueryData(['snapshot-meta', 'i-3'])).toBeDefined()
    // Chat only for the focused id.
    expect(client.getQueryData(['snapshot-chat', 'i-2'])).toBeDefined()
    expect(client.getQueryData(['snapshot-chat', 'i-1'])).toBeUndefined()
  })

  it('seeds useActiveInstance from instances/list focusedId when no local choice exists', async() => {
    invoke.mockImplementation((command: string) => {
      if (command === TauriCommand.InstancesList) {
        return Promise.resolve({
          instances: [
            { agentId: 'a', instanceId: 'i-1' },
            { agentId: 'a', instanceId: 'i-2' }
          ],
          focusedId: 'i-2'
        })
      }

      if (command === TauriCommand.InstanceSnapshotMeta) {
        return Promise.resolve({ mcpsCount: 0, usage: { used: 0, size: 0 } })
      }

      if (command === TauriCommand.InstanceSnapshotChat) {
        return Promise.resolve({ items: [], hasMore: false })
      }

      return Promise.reject(new Error(`unexpected command ${command}`))
    })
    const client = buildClient()

    // No local focus → daemon's focusedId fills the slot.
    await brimSync(client, undefined)

    expect(useActiveInstance().id.value).toBe('i-2')
    // Chat for the daemon-reported focus is also primed.
    expect(client.getQueryData(['snapshot-chat', 'i-2'])).toBeDefined()
  })

  it('preserves caller-supplied local focus over the daemon focusedId', async() => {
    invoke.mockImplementation((command: string) => {
      if (command === TauriCommand.InstancesList) {
        return Promise.resolve({
          instances: [
            { agentId: 'a', instanceId: 'i-1' },
            { agentId: 'a', instanceId: 'i-2' }
          ],
          focusedId: 'i-2'
        })
      }

      if (command === TauriCommand.InstanceSnapshotMeta) {
        return Promise.resolve({ mcpsCount: 0, usage: { used: 0, size: 0 } })
      }

      if (command === TauriCommand.InstanceSnapshotChat) {
        return Promise.resolve({ items: [], hasMore: false })
      }

      return Promise.reject(new Error(`unexpected command ${command}`))
    })
    const client = buildClient()

    await brimSync(client, 'i-1')

    // Chat was prefetched for the local choice, NOT the daemon focus.
    expect(client.getQueryData(['snapshot-chat', 'i-1'])).toBeDefined()
    expect(client.getQueryData(['snapshot-chat', 'i-2'])).toBeUndefined()
    // Local focus seeds the empty active-instance slot.
    expect(useActiveInstance().id.value).toBe('i-1')
  })

  it('skips chat prefetch when no focused id is supplied', async() => {
    invoke.mockImplementation((command: string) => {
      if (command === TauriCommand.InstancesList) {
        return Promise.resolve({
          instances: [
            {
              agentId: 'a',
              instanceId: 'i-1',
              name: undefined
            }
          ]
        })
      }

      if (command === TauriCommand.InstanceSnapshotMeta) {
        return Promise.resolve({ mcpsCount: 0, usage: { used: 0, size: 0 } })
      }

      return Promise.reject(new Error(`unexpected command ${command}`))
    })
    const client = buildClient()

    await brimSync(client, undefined)

    const chatCalls = invoke.mock.calls.filter((c) => c[0] === TauriCommand.InstanceSnapshotChat)

    expect(chatCalls).toHaveLength(0)
  })

  it('returns silently when instances_list fails — partial sync is tolerated', async() => {
    invoke.mockRejectedValueOnce(new Error('boom'))
    const client = buildClient()

    await brimSync(client, 'i-1')

    // No prefetches fired because the list never resolved.
    const calls = invoke.mock.calls.map((c) => c[0])

    expect(calls.filter((c) => c === TauriCommand.InstanceSnapshotMeta)).toHaveLength(0)
    expect(calls.filter((c) => c === TauriCommand.InstanceSnapshotChat)).toHaveLength(0)
  })
})

describe('useFocusPrefetch.start', () => {
  it('prefetches meta + chat on acp:instances-focused', async() => {
    invoke.mockImplementation((command: string) => {
      if (command === TauriCommand.InstanceSnapshotMeta) {
        return Promise.resolve({ mcpsCount: 0, usage: { used: 0, size: 0 } })
      }

      if (command === TauriCommand.InstanceSnapshotChat) {
        return Promise.resolve({ items: [], hasMore: false })
      }

      return Promise.reject(new Error(`unexpected command ${command}`))
    })
    const client = buildClient()
    const api = useFocusPrefetch(client)
    const stop = await api.start()

    const cb = listeners.get(TauriEvent.AcpInstancesFocused)

    expect(cb).toBeDefined()
    cb!({ payload: { instanceId: 'i-new' } })

    // Wait for the in-flight prefetches to settle.
    await new Promise((resolve) => setTimeout(resolve, 0))
    await new Promise((resolve) => setTimeout(resolve, 0))

    const calls = invoke.mock.calls.map((c) => c[0])

    expect(calls).toContain(TauriCommand.InstanceSnapshotMeta)
    expect(calls).toContain(TauriCommand.InstanceSnapshotChat)
    expect(client.getQueryData(['snapshot-meta', 'i-new'])).toBeDefined()
    expect(client.getQueryData(['snapshot-chat', 'i-new'])).toBeDefined()

    stop()
  })

  it('skips prefetch when acp:instances-focused fires with no instanceId', async() => {
    const client = buildClient()
    const api = useFocusPrefetch(client)
    const stop = await api.start()

    const cb = listeners.get(TauriEvent.AcpInstancesFocused)

    cb!({ payload: { instanceId: undefined } })

    await new Promise((resolve) => setTimeout(resolve, 0))

    expect(invoke).not.toHaveBeenCalled()
    stop()
  })

  it('prefetches meta + chat on acp:instances-changed for every cache-miss id', async() => {
    // Each new id triggers BOTH a meta and a chat prefetch — the
    // captain may navigate to any of these momentarily, and the
    // chat prefetch covers daemon-side or peer-client spawns (nvim,
    // ctl, session_load) whose replay events would otherwise hit
    // the patcher's "no cache → drop" guard.
    invoke.mockImplementation((command: string) => {
      if (command === TauriCommand.InstanceSnapshotMeta) {
        return Promise.resolve({ mcpsCount: 0, usage: { used: 0, size: 0 } })
      }

      if (command === TauriCommand.InstanceSnapshotChat) {
        return Promise.resolve({ items: [], hasMore: false })
      }

      return Promise.reject(new Error(`unexpected command ${command}`))
    })
    const client = buildClient()
    const api = useFocusPrefetch(client)
    const stop = await api.start()

    const cb = listeners.get(TauriEvent.AcpInstancesChanged)

    expect(cb).toBeDefined()
    cb!({
      payload: {
        instanceIds: ['i-a', 'i-b'],
        focusedId: 'i-a'
      }
    })

    await new Promise((resolve) => setTimeout(resolve, 0))
    await new Promise((resolve) => setTimeout(resolve, 0))

    const metaCalls = invoke.mock.calls.filter((c) => c[0] === TauriCommand.InstanceSnapshotMeta)
    const chatCalls = invoke.mock.calls.filter((c) => c[0] === TauriCommand.InstanceSnapshotChat)

    expect(metaCalls).toHaveLength(2)
    expect(chatCalls).toHaveLength(2)
    expect(client.getQueryData(['snapshot-meta', 'i-a'])).toBeDefined()
    expect(client.getQueryData(['snapshot-meta', 'i-b'])).toBeDefined()
    expect(client.getQueryData(['snapshot-chat', 'i-a'])).toBeDefined()
    expect(client.getQueryData(['snapshot-chat', 'i-b'])).toBeDefined()

    stop()
  })

  it('skips chat prefetch on instances-changed when the cache is already hydrated', async() => {
    // Boot snapshot / prior focus event already seeded the cache for
    // some ids; instances-changed firing for those same ids must NOT
    // re-fetch. TanStack's queryKey dedup would handle parallel
    // in-flight fetches, but a populated cache shouldn't get a stale
    // refresh either. The cache-miss gate keeps the prefetch tight.
    invoke.mockImplementation((command: string) => {
      if (command === TauriCommand.InstanceSnapshotMeta) {
        return Promise.resolve({ mcpsCount: 0, usage: { used: 0, size: 0 } })
      }

      if (command === TauriCommand.InstanceSnapshotChat) {
        return Promise.resolve({ items: [], hasMore: false })
      }

      return Promise.reject(new Error(`unexpected command ${command}`))
    })
    const client = buildClient()

    // Pre-seed `i-warm` as if boot snapshot landed it.
    client.setQueryData(['snapshot-chat', 'i-warm'], { pages: [{ items: [], hasMore: false }], pageParams: [undefined] })

    const api = useFocusPrefetch(client)
    const stop = await api.start()

    const cb = listeners.get(TauriEvent.AcpInstancesChanged)

    cb!({
      payload: {
        instanceIds: ['i-warm', 'i-cold'],
        focusedId: 'i-warm'
      }
    })

    await new Promise((resolve) => setTimeout(resolve, 0))
    await new Promise((resolve) => setTimeout(resolve, 0))

    const chatCalls = invoke.mock.calls.filter((c) => c[0] === TauriCommand.InstanceSnapshotChat)

    // Only i-cold gets a fetch — i-warm is already in cache.
    expect(chatCalls).toHaveLength(1)
    expect(chatCalls[0]?.[1]).toMatchObject({ instanceId: 'i-cold' })

    stop()
  })

  it('teardown removes the focused / changed listeners', async() => {
    const client = buildClient()
    const api = useFocusPrefetch(client)
    const stop = await api.start()

    expect(listeners.has(TauriEvent.AcpInstancesFocused)).toBe(true)
    expect(listeners.has(TauriEvent.AcpInstancesChanged)).toBe(true)

    stop()

    expect(listeners.has(TauriEvent.AcpInstancesFocused)).toBe(false)
    expect(listeners.has(TauriEvent.AcpInstancesChanged)).toBe(false)
  })
})
