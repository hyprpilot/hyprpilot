/**
 * Pin the cache-seed-before-replay behavior in `load()`.
 *
 * Without the seed, the daemon's session/load replay fires
 * `acp:transcript` events whose target instance id doesn't yet
 * have a cache key. `transcript-patcher.flushPatchesFor` then
 * silently drops the batch (its "no cache yet" guard) and the
 * captain's restored session paints empty — symptom captain
 * described as "remote replay isn't replaying properly while
 * nvim works fine".
 *
 * The fix: seed `['snapshot-chat', target]` with an empty head
 * page BEFORE invoking SessionLoad. The patcher then finds the
 * cache key, treats it as the first page, and appends the live
 * replay items to `head.items`.
 */

import { QueryClient, VueQueryPlugin } from '@tanstack/vue-query'
import { mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { computed, defineComponent, h, type Ref } from 'vue'

import { useSessionHistory } from './use-session-history'

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }))

vi.mock('@ipc', async() => ({
  ...(await vi.importActual<object>('@ipc')),
  invoke: (command: string, args?: Record<string, unknown>) => invokeMock(command, args)
}))

// Stub crypto.randomUUID so the test can assert the seed lands on
// the SAME id passed to SessionLoad.
const TARGET_ID = 'aaaaaaaa-1111-2222-3333-444444444444'

beforeEach(() => {
  invokeMock.mockReset()
  // Vitest jsdom env provides crypto, but not randomUUID — stub it.
  Object.defineProperty(globalThis.crypto, 'randomUUID', {
    value: () => TARGET_ID,
    configurable: true,
    writable: true
  })
})

function mountWithClient(client: QueryClient, run: (api: ReturnType<typeof useSessionHistory>) => void): void {
  const host = defineComponent({
    setup() {
      const agent = computed<string | undefined>(() => 'claude-code')
      const profile = computed<string | undefined>(() => 'personal/claude/opus')
      const api = useSessionHistory(agent as unknown as Ref<string | undefined>, profile as unknown as Ref<string | undefined>)

      run(api)

      return () => h('div')
    }
  })

  mount(host, {
    global: {
      plugins: [[VueQueryPlugin, { queryClient: client }]]
    }
  })
}

describe('useSessionHistory.load', () => {
  it('seeds an empty head page in the chat cache BEFORE invoking SessionLoad', async() => {
    invokeMock.mockResolvedValue(undefined)
    const client = new QueryClient()

    let api: ReturnType<typeof useSessionHistory> | undefined

    mountWithClient(client, (a) => {
      api = a
    })

    expect(api).toBeDefined()
    // Cache is empty before load fires.
    expect(client.getQueryData(['snapshot-chat', TARGET_ID])).toBeUndefined()

    await api!.load('session-xyz', '/home/cenk/dev/foo')

    const cached = client.getQueryData(['snapshot-chat', TARGET_ID]) as { pages: { items: unknown[]; hasMore: boolean }[] } | undefined

    expect(cached?.pages).toHaveLength(1)
    expect(cached?.pages[0]?.items).toEqual([])
    expect(cached?.pages[0]?.hasMore).toBe(false)
  })

  it('seed is in place by the time SessionLoad fires (so the replay-events patcher finds a non-empty cache)', async() => {
    let seedAtInvoke: unknown

    invokeMock.mockImplementation((_command: string, _args: unknown) => {
      seedAtInvoke = new QueryClient() // placeholder — captured below

      return Promise.resolve(undefined)
    })
    const client = new QueryClient()

    invokeMock.mockImplementation((_command: string, _args: unknown) => {
      seedAtInvoke = client.getQueryData(['snapshot-chat', TARGET_ID])

      return Promise.resolve(undefined)
    })

    let api: ReturnType<typeof useSessionHistory> | undefined

    mountWithClient(client, (a) => {
      api = a
    })

    await api!.load('session-xyz')

    expect(seedAtInvoke).toBeDefined()
    expect((seedAtInvoke as { pages: { items: unknown[] }[] }).pages[0]?.items).toEqual([])
  })
})
