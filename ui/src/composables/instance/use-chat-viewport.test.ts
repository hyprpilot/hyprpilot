import { QueryClient, VueQueryPlugin } from '@tanstack/vue-query'
import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { computed, defineComponent, h, ref, type Ref } from 'vue'

import { MAX_PAGES_KEPT, useChatViewport, type UseChatViewportApi } from './use-chat-viewport'
import { pushPermissionRequest, resetPermissions, usePermissions } from './use-permissions'
import type { InstanceId } from '../chrome/use-active-instance'
import { TauriEvent, TranscriptItemKind, type ChatSnapshot, type MetaSnapshot, type SeqTranscriptItem } from '@ipc'

const { invoke, listeners } = vi.hoisted(() => ({
  invoke: vi.fn(),
  listeners: new Map<string, (payload: { payload: unknown }) => void>()
}))

vi.mock('@ipc/bridge', async() => ({
  ...(await vi.importActual<object>('@ipc/bridge')),
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
        // Non-zero gcTime so `setQueryData` on an unobserved key
        // (e.g. seeding the meta snapshot the per-focus pull would
        // populate later) doesn't immediately GC the entry.
        // 60s is well past test-suite duration.
        gcTime: 60_000,
        staleTime: 0
      }
    }
  })
}

function transcriptText(seq: number, text: string, kind: TranscriptItemKind = TranscriptItemKind.AgentText): SeqTranscriptItem {
  return { seq, item: { kind, text } as never }
}

function chatPage(items: SeqTranscriptItem[], hasMore = false): ChatSnapshot {
  return {
    items,
    oldestSeq: items[0]?.seq,
    latestSeq: items[items.length - 1]?.seq,
    hasMore
  }
}

interface MountResult {
  api: UseChatViewportApi
  queryClient: QueryClient
  unmount: () => void
}

function mountViewport(idRef: Ref<InstanceId | undefined>): MountResult {
  let captured: UseChatViewportApi | undefined
  const TestComponent = defineComponent({
    setup() {
      const id = computed(() => idRef.value)

      captured = useChatViewport(id)

      return () => h('div')
    }
  })
  const queryClient = buildClient()
  const wrapper = mount(TestComponent, {
    global: { plugins: [[VueQueryPlugin, { queryClient }]] }
  })

  if (!captured) {
    throw new Error('useChatViewport never returned')
  }

  return {
    api: captured,
    queryClient,
    unmount: () => wrapper.unmount()
  }
}

beforeEach(() => {
  invoke.mockReset()
  listeners.clear()
})

describe('useChatViewport', () => {
  it('flattens cached pages oldest-first and exposes latestSeq', async() => {
    invoke.mockResolvedValueOnce(chatPage([transcriptText(150, 'hello'), transcriptText(151, 'world')], false))
    const id = ref<InstanceId | undefined>('i-1')
    const { api, unmount } = mountViewport(id)

    await flushPromises()
    await flushPromises()

    expect(api.items.value.map((it) => it.seq)).toEqual([150, 151])
    expect(api.latestSeq.value).toBe(151)
    expect(api.hasNextPage.value).toBe(false)
    unmount()
  })

  it('live-event patcher appends a new agent_text onto the latest page', async() => {
    invoke.mockResolvedValueOnce(chatPage([transcriptText(50, 'first')], false))
    const id = ref<InstanceId | undefined>('i-1')
    const { api, unmount } = mountViewport(id)

    await flushPromises()
    await flushPromises()

    expect(api.items.value).toHaveLength(1)

    // Fire a live `acp:transcript` event for the focused instance.
    const cb = listeners.get(TauriEvent.AcpTranscript)

    expect(cb).toBeDefined()
    cb!({
      payload: {
        agentId: 'a',
        instanceId: 'i-1',
        sessionId: 's',
        item: { kind: TranscriptItemKind.AgentText, text: 'streamed' }
      } as never
    })
    await flushPromises()

    expect(api.items.value).toHaveLength(2)
    const tail = api.items.value[1]!

    expect(tail.item).toEqual({ kind: TranscriptItemKind.AgentText, text: 'streamed' })
    // Local seq advanced past the cached latestSeq.
    expect(tail.seq).toBeGreaterThan(50)
    unmount()
  })

  it('live-event patcher merges a tool_call_update onto an existing tool_call by id', async() => {
    const initial: SeqTranscriptItem = {
      seq: 10,
      item: {
        kind: TranscriptItemKind.ToolCall,
        id: 'tc-1',
        toolKind: 'bash',
        title: 'echo',
        state: 'running',
        content: [],
        formatted: {
          title: 'echo', stats: [], fields: []
        },
        startedAtMs: 1000
      } as never
    }

    invoke.mockResolvedValueOnce(chatPage([initial], false))
    const id = ref<InstanceId | undefined>('i-1')
    const { api, unmount } = mountViewport(id)

    await flushPromises()
    await flushPromises()

    expect(api.items.value).toHaveLength(1)

    const cb = listeners.get(TauriEvent.AcpTranscript)

    cb!({
      payload: {
        agentId: 'a',
        instanceId: 'i-1',
        sessionId: 's',
        item: {
          kind: TranscriptItemKind.ToolCallUpdate,
          id: 'tc-1',
          state: 'completed',
          content: [{ kind: 'text', text: 'hi' }],
          formatted: {
            title: 'echo', stats: [], fields: []
          },
          startedAtMs: 1000,
          completedAtMs: 1100
        } as never
      } as never
    })
    await flushPromises()

    // Item count unchanged — merge in place.
    expect(api.items.value).toHaveLength(1)
    const merged = api.items.value[0]!

    expect((merged.item as { state: string }).state).toBe('completed')
    expect((merged.item as { completedAtMs: number }).completedAtMs).toBe(1100)
    unmount()
  })

  it('permission-resolved patches the meta cache to drop the matching pending entry', async() => {
    invoke.mockResolvedValueOnce(chatPage([], false))
    const id = ref<InstanceId | undefined>('i-1')
    const { api, queryClient, unmount } = mountViewport(id)

    // Seed the meta cache BEFORE mount-time async work completes so
    // the listener has a populated cache to mutate when it fires.
    const meta: MetaSnapshot = {
      mcpsCount: 0,
      usage: { used: 0, size: 0 },
      pendingPermissions: [
        {
          requestId: 'r-1',
          tool: 'bash',
          options: []
        },
        {
          requestId: 'r-2',
          tool: 'edit',
          options: []
        }
      ]
    }

    queryClient.setQueryData(['snapshot-meta', 'i-1'], meta)

    // Wait until the composable's async listen registration completes
    // — `void (async () => { ... })()` doesn't block mount, so the
    // listener may not be in the map immediately. Loop with
    // flushPromises until it shows up (bounded so a real bug fails
    // fast). Drains both `listen()` awaits + the initial chat fetch.
    let cb: ((p: { payload: unknown }) => void) | undefined

    for (let i = 0; i < 20 && cb === undefined; i += 1) {
      await flushPromises()
      cb = listeners.get(TauriEvent.AcpPermissionResolved)
    }
    expect(cb).toBeDefined()

    // Sanity: the seed actually landed in the queryClient the
    // composable shares with the test (via per-mount VueQueryPlugin).
    expect(queryClient.getQueryData<MetaSnapshot>(['snapshot-meta', 'i-1'])?.pendingPermissions?.length).toBe(2)

    cb!({
      payload: {
        instanceId: 'i-1', requestId: 'r-1', optionId: 'allow-once'
      }
    })
    await flushPromises()

    const next = queryClient.getQueryData<MetaSnapshot>(['snapshot-meta', 'i-1'])

    expect(next?.pendingPermissions?.map((p) => p.requestId)).toEqual(['r-2'])
    // viewport's own items shape didn't change — snapshot cache is the only mutation.
    expect(api.items.value).toHaveLength(0)
    unmount()
  })

  it('page-trim drops oldest pages when stuck-at-bottom and cache exceeds MAX_PAGES_KEPT', async() => {
    invoke
      .mockResolvedValueOnce(chatPage([transcriptText(200, 'p0')], true))
      .mockResolvedValueOnce(chatPage([transcriptText(150, 'p1')], true))
      .mockResolvedValueOnce(chatPage([transcriptText(100, 'p2')], true))
      .mockResolvedValueOnce(chatPage([transcriptText(50, 'p3')], false))
    const id = ref<InstanceId | undefined>('i-1')
    const { api, unmount } = mountViewport(id)

    await flushPromises()
    await flushPromises()
    await api.fetchNextPage()
    await flushPromises()
    await api.fetchNextPage()
    await flushPromises()
    await api.fetchNextPage()
    await flushPromises()

    // Four pages cached → flattened items count is 4 (one item per
    // mocked page) before the trim.
    expect(api.items.value.map((it) => it.seq).sort((a, b) => a - b)).toEqual([50, 100, 150, 200])

    api.onStuckChange(true)
    await flushPromises()

    // After the trim only the newest MAX_PAGES_KEPT pages remain —
    // the oldest page (seq 50) gets dropped. Items run newest seqs only.
    const seqs = api.items.value.map((it) => it.seq).sort((a, b) => a - b)

    expect(seqs).toHaveLength(MAX_PAGES_KEPT)
    expect(seqs).toContain(200)
    expect(seqs).not.toContain(50)
    unmount()
  })

  it('does not trim when stuck-at-bottom but cache is at or below MAX_PAGES_KEPT', async() => {
    invoke.mockResolvedValueOnce(chatPage([transcriptText(50, 'p0')], false))
    const id = ref<InstanceId | undefined>('i-1')
    const { api, unmount } = mountViewport(id)

    await flushPromises()
    await flushPromises()

    expect(api.items.value).toHaveLength(1)

    api.onStuckChange(true)
    await flushPromises()

    expect(api.items.value).toHaveLength(1)
    unmount()
  })

  it('isFetchingNextPage flips while a backward fetch is in flight', async() => {
    invoke.mockResolvedValueOnce(chatPage([transcriptText(100, 'p0')], true))
    const id = ref<InstanceId | undefined>('i-1')
    const { api, unmount } = mountViewport(id)

    await flushPromises()
    await flushPromises()

    expect(api.isFetchingNextPage.value).toBe(false)
    expect(api.hasNextPage.value).toBe(true)

    let resolveNext: (v: ChatSnapshot) => void = () => {}

    invoke.mockImplementationOnce(
      () =>
        new Promise<ChatSnapshot>((resolve) => {
          resolveNext = resolve
        })
    )

    void api.fetchNextPage()
    await flushPromises()
    expect(api.isFetchingNextPage.value).toBe(true)

    resolveNext(chatPage([transcriptText(50, 'p1')], false))
    await flushPromises()
    expect(api.isFetchingNextPage.value).toBe(false)
    unmount()
  })

  it('permission-resolved clears the matching pending row from the permissions store', async() => {
    invoke.mockResolvedValueOnce(chatPage([], false))
    const id = ref<InstanceId | undefined>('i-1')
    const { unmount } = mountViewport(id)

    // Seed the pending permissions store on the focused instance and a sibling.
    resetPermissions('i-1')
    resetPermissions('i-OTHER')
    pushPermissionRequest('i-1', 's-a', {
      agentId: 'agent',
      requestId: 'req-1',
      tool: 'bash',
      kind: 'execute',
      args: 'echo hi',
      options: [],
      formatted: {
        title: 'bash',
        stats: [],
        fields: []
      }
    })
    pushPermissionRequest('i-OTHER', 's-b', {
      agentId: 'agent',
      requestId: 'req-cross',
      tool: 'bash',
      kind: 'execute',
      args: 'echo on other instance',
      options: [],
      formatted: {
        title: 'bash',
        stats: [],
        fields: []
      }
    })

    let cb: ((p: { payload: unknown }) => void) | undefined

    for (let i = 0; i < 20 && cb === undefined; i += 1) {
      await flushPromises()
      cb = listeners.get(TauriEvent.AcpPermissionResolved)
    }
    expect(cb).toBeDefined()

    // Resolve the focused instance's row.
    cb!({
      payload: {
        instanceId: 'i-1', requestId: 'req-1', optionId: 'allow-once'
      }
    })
    await flushPromises()

    expect(usePermissions('i-1').rowQueue.value).toHaveLength(0)
    // Sibling row untouched — the listener addresses by payload.instanceId.
    expect(usePermissions('i-OTHER').rowQueue.value.map((v) => v.request.requestId)).toEqual(['req-cross'])

    // Cross-instance resolution still clears even when the viewport
    // is focused elsewhere — captain answered on remote, desktop's
    // store drops the row.
    cb!({
      payload: {
        instanceId: 'i-OTHER', requestId: 'req-cross', optionId: 'reject-once'
      }
    })
    await flushPromises()

    expect(usePermissions('i-OTHER').rowQueue.value).toHaveLength(0)
    unmount()
  })

  it('ignores live events for a different instance id', async() => {
    invoke.mockResolvedValueOnce(chatPage([transcriptText(10, 'a')], false))
    const id = ref<InstanceId | undefined>('i-1')
    const { api, unmount } = mountViewport(id)

    await flushPromises()
    await flushPromises()

    const cb = listeners.get(TauriEvent.AcpTranscript)

    cb!({
      payload: {
        agentId: 'a',
        instanceId: 'i-OTHER',
        sessionId: 's',
        item: { kind: TranscriptItemKind.AgentText, text: 'noise' }
      } as never
    })
    await flushPromises()

    expect(api.items.value).toHaveLength(1)
    unmount()
  })

  // ── Regression tests for the burst-replay bugs ────────────────────
  //
  // These cover the failure modes captured in earlier tmux traces:
  //
  //  1. session/load replay storm — hundreds of `acp:transcript`
  //     notifications fire in <2ms. Each must NOT trigger a separate
  //     `setQueryData` cycle. The microtask-batched `flushPatches`
  //     should coalesce the burst into one head-page update.
  //
  //  2. live-patched items must keep `payload.turnId` so
  //     `block.turnId` resolves to the same `turn:<id>` group key
  //     the snapshot path uses. Dropping the turnId here was what
  //     broke header chip rendering during streaming (commit b8c695a).

  it('coalesces a burst of live patches into one head-page update', async() => {
    invoke.mockResolvedValueOnce(chatPage([transcriptText(0, 'seed')], false))
    const id = ref<InstanceId | undefined>('i-1')
    const { api, queryClient, unmount } = mountViewport(id)

    await flushPromises()
    await flushPromises()

    const cb = listeners.get(TauriEvent.AcpTranscript)

    expect(cb).toBeDefined()

    // Spy on setQueryData so we can count actual updates to the
    // chat-page cache key. Microtask batching should fuse the whole
    // burst into a single setQueryData call.
    const original = queryClient.setQueryData.bind(queryClient)
    let chatUpdates = 0

    vi.spyOn(queryClient, 'setQueryData').mockImplementation(((key: unknown, updater: unknown, opts: unknown) => {
      if (Array.isArray(key) && key[0] === 'snapshot-chat') {
        chatUpdates += 1
      }

      return (original as unknown as (k: unknown, u: unknown, o: unknown) => unknown)(key, updater, opts)
    }) as unknown as typeof queryClient.setQueryData)

    // Fire 100 synchronous events — replicates the session/load
    // replay shape (hundreds of notifications in one tick).
    const N = 100

    for (let i = 0; i < N; i += 1) {
      cb!({
        payload: {
          agentId: 'a',
          instanceId: 'i-1',
          sessionId: 's',
          turnId: 't-1',
          item: { kind: TranscriptItemKind.AgentText, text: `chunk-${i}` }
        } as never
      })
    }
    // Microtask boundary — flushPatches drains the queue.
    await flushPromises()

    // All items landed.
    expect(api.items.value).toHaveLength(1 + N)
    // ALL items came from a SINGLE setQueryData call. Without
    // batching, this would be N (one per patch).
    expect(chatUpdates).toBe(1)
    unmount()
  })

  it('preserves payload.turnId on live-patched items', async() => {
    invoke.mockResolvedValueOnce(chatPage([], false))
    const id = ref<InstanceId | undefined>('i-1')
    const { api, unmount } = mountViewport(id)

    await flushPromises()
    await flushPromises()

    const cb = listeners.get(TauriEvent.AcpTranscript)

    cb!({
      payload: {
        agentId: 'a',
        instanceId: 'i-1',
        sessionId: 's',
        turnId: 'turn-abc',
        item: { kind: TranscriptItemKind.AgentText, text: 'hello' }
      } as never
    })
    await flushPromises()

    expect(api.items.value).toHaveLength(1)
    expect(api.items.value[0]?.turnId).toBe('turn-abc')
    unmount()
  })

  it('drops queued patches on unmount so the next microtask can\'t mutate a torn-down queryClient', async() => {
    invoke.mockResolvedValueOnce(chatPage([], false))
    const id = ref<InstanceId | undefined>('i-1')
    const { unmount, queryClient } = mountViewport(id)

    await flushPromises()
    await flushPromises()

    const cb = listeners.get(TauriEvent.AcpTranscript)
    const before = queryClient.getQueryData(['snapshot-chat', 'i-1'])

    // Queue a patch but unmount BEFORE the microtask runs.
    cb?.({
      payload: {
        agentId: 'a',
        instanceId: 'i-1',
        sessionId: 's',
        turnId: 't',
        item: { kind: TranscriptItemKind.AgentText, text: 'after-unmount' }
      } as never
    })
    unmount()
    await flushPromises()

    // Cache untouched — the flush early-returned on the stopped
    // composable.
    const after = queryClient.getQueryData(['snapshot-chat', 'i-1'])

    expect(after).toBe(before)
  })
})
