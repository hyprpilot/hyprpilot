import { QueryClient } from '@tanstack/vue-query'
import { flushPromises } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { __resetTranscriptPatcherForTests, startTranscriptPatcher } from './transcript-patcher'
import { pushPermissionRequest, resetPermissions, usePermissions } from './use-permissions'
import { TauriEvent, TranscriptItemKind, type ChatSnapshot, type MetaSnapshot, type SeqTranscriptItem } from '@ipc'

const { listeners } = vi.hoisted(() => ({
  listeners: new Map<string, (payload: { payload: unknown }) => void>()
}))

vi.mock('@ipc', async() => ({
  ...(await vi.importActual<object>('@ipc')),
  listen: (event: string, cb: (payload: { payload: unknown }) => void) => {
    listeners.set(event, cb)

    return Promise.resolve(() => listeners.delete(event))
  }
}))

function chatPage(items: SeqTranscriptItem[], hasMore = false): ChatSnapshot {
  return {
    items,
    oldestSeq: items[0]?.seq,
    latestSeq: items[items.length - 1]?.seq,
    hasMore
  }
}

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

beforeEach(async() => {
  listeners.clear()
  __resetTranscriptPatcherForTests()
})

afterEach(() => {
  __resetTranscriptPatcherForTests()
})

describe('transcript-patcher singleton', () => {
  it('appends a new agent_text onto the latest page', async() => {
    const queryClient = buildClient()

    queryClient.setQueryData(['snapshot-chat', 'i-1'], {
      pages: [chatPage([{ seq: 50, item: { kind: TranscriptItemKind.AgentText, text: 'first' } as never }])],
      pageParams: [undefined]
    })

    await startTranscriptPatcher(queryClient)
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

    const data = queryClient.getQueryData<{ pages: ChatSnapshot[] }>(['snapshot-chat', 'i-1'])

    expect(data?.pages[0].items).toHaveLength(2)
    expect(data?.pages[0].items[1].item).toEqual({ kind: TranscriptItemKind.AgentText, text: 'streamed' })
  })

  it('merges a tool_call_update onto an existing tool_call by id', async() => {
    const queryClient = buildClient()
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
          title: 'echo',
          stats: [],
          fields: []
        },
        startedAtMs: 1000
      } as never
    }

    queryClient.setQueryData(['snapshot-chat', 'i-1'], {
      pages: [chatPage([initial])],
      pageParams: [undefined]
    })

    await startTranscriptPatcher(queryClient)
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
            title: 'echo',
            stats: [],
            fields: []
          },
          startedAtMs: 1000,
          completedAtMs: 1100
        } as never
      } as never
    })
    await flushPromises()

    const items = queryClient.getQueryData<{ pages: ChatSnapshot[] }>(['snapshot-chat', 'i-1'])?.pages[0].items

    expect(items).toHaveLength(1)
    expect((items![0].item as { state: string }).state).toBe('completed')
    expect((items![0].item as { completedAtMs: number }).completedAtMs).toBe(1100)
  })

  it('concatenates tool_call_update content (does NOT replace)', async() => {
    const queryClient = buildClient()
    const initial: SeqTranscriptItem = {
      seq: 10,
      item: {
        kind: TranscriptItemKind.ToolCall,
        id: 'tc-1',
        toolKind: 'bash',
        title: 'echo',
        state: 'running',
        content: [{ kind: 'text', text: 'first' }],
        formatted: {
          title: 'echo',
          stats: [],
          fields: []
        },
        startedAtMs: 1000
      } as never
    }

    queryClient.setQueryData(['snapshot-chat', 'i-1'], { pages: [chatPage([initial])], pageParams: [undefined] })

    await startTranscriptPatcher(queryClient)
    const cb = listeners.get(TauriEvent.AcpTranscript)

    cb!({
      payload: {
        agentId: 'a',
        instanceId: 'i-1',
        sessionId: 's',
        item: {
          kind: TranscriptItemKind.ToolCallUpdate,
          id: 'tc-1',
          content: [{ kind: 'text', text: 'second' }],
          formatted: {
            title: 'echo',
            stats: [],
            fields: []
          },
          startedAtMs: 1000
        } as never
      } as never
    })
    cb!({
      payload: {
        agentId: 'a',
        instanceId: 'i-1',
        sessionId: 's',
        item: {
          kind: TranscriptItemKind.ToolCallUpdate,
          id: 'tc-1',
          content: [{ kind: 'text', text: 'third' }],
          formatted: {
            title: 'echo',
            stats: [],
            fields: []
          },
          startedAtMs: 1000
        } as never
      } as never
    })
    await flushPromises()

    const items = queryClient.getQueryData<{ pages: ChatSnapshot[] }>(['snapshot-chat', 'i-1'])?.pages[0].items
    const merged = items![0].item as { content: { text: string }[] }

    expect(merged.content.map((c) => c.text)).toEqual(['first', 'second', 'third'])
  })

  it('preserves turnId on tool_call_update merge', async() => {
    const queryClient = buildClient()
    const initial: SeqTranscriptItem = {
      seq: 10,
      turnId: 'turn-A',
      item: {
        kind: TranscriptItemKind.ToolCall,
        id: 'tc-1',
        toolKind: 'bash',
        title: 'echo',
        state: 'running',
        content: [],
        formatted: {
          title: 'echo',
          stats: [],
          fields: []
        },
        startedAtMs: 1000
      } as never
    }

    queryClient.setQueryData(['snapshot-chat', 'i-1'], { pages: [chatPage([initial])], pageParams: [undefined] })

    await startTranscriptPatcher(queryClient)
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
          content: [],
          formatted: {
            title: 'echo',
            stats: [],
            fields: []
          },
          startedAtMs: 1000,
          completedAtMs: 1100
        } as never
      } as never
    })
    await flushPromises()

    const items = queryClient.getQueryData<{ pages: ChatSnapshot[] }>(['snapshot-chat', 'i-1'])?.pages[0].items

    expect(items![0].turnId).toBe('turn-A')
    expect((items![0].item as { state: string }).state).toBe('completed')
  })

  it('preserves payload.turnId on live-patched items', async() => {
    const queryClient = buildClient()

    queryClient.setQueryData(['snapshot-chat', 'i-1'], { pages: [chatPage([])], pageParams: [undefined] })

    await startTranscriptPatcher(queryClient)
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

    const items = queryClient.getQueryData<{ pages: ChatSnapshot[] }>(['snapshot-chat', 'i-1'])?.pages[0].items

    expect(items).toHaveLength(1)
    expect(items![0].turnId).toBe('turn-abc')
  })

  it('coalesces a burst of live patches into one head-page update', async() => {
    const queryClient = buildClient()

    queryClient.setQueryData(['snapshot-chat', 'i-1'], {
      pages: [chatPage([{ seq: 0, item: { kind: TranscriptItemKind.AgentText, text: 'seed' } as never }])],
      pageParams: [undefined]
    })

    await startTranscriptPatcher(queryClient)
    const cb = listeners.get(TauriEvent.AcpTranscript)

    const original = queryClient.setQueryData.bind(queryClient)
    let chatUpdates = 0

    vi.spyOn(queryClient, 'setQueryData').mockImplementation(((key: unknown, updater: unknown, opts: unknown) => {
      if (Array.isArray(key) && key[0] === 'snapshot-chat') {
        chatUpdates += 1
      }

      return (original as unknown as (k: unknown, u: unknown, o: unknown) => unknown)(key, updater, opts)
    }) as unknown as typeof queryClient.setQueryData)

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
    await flushPromises()

    const items = queryClient.getQueryData<{ pages: ChatSnapshot[] }>(['snapshot-chat', 'i-1'])?.pages[0].items

    expect(items).toHaveLength(1 + N)
    expect(chatUpdates).toBe(1)
  })

  it('skips live events when no cache exists for the instance', async() => {
    const queryClient = buildClient()

    await startTranscriptPatcher(queryClient)
    const cb = listeners.get(TauriEvent.AcpTranscript)

    cb!({
      payload: {
        agentId: 'a',
        instanceId: 'i-1',
        sessionId: 's',
        item: { kind: TranscriptItemKind.AgentText, text: 'before-snapshot' }
      } as never
    })
    await flushPromises()

    expect(queryClient.getQueryData(['snapshot-chat', 'i-1'])).toBeUndefined()
  })

  it('patches the meta cache to drop the matching pending entry on permission-resolved', async() => {
    const queryClient = buildClient()
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
    } as MetaSnapshot

    queryClient.setQueryData(['snapshot-meta', 'i-1'], meta)

    await startTranscriptPatcher(queryClient)
    const cb = listeners.get(TauriEvent.AcpPermissionResolved)

    expect(cb).toBeDefined()

    cb!({
      payload: {
        instanceId: 'i-1',
        requestId: 'r-1',
        optionId: 'allow-once'
      }
    })
    await flushPromises()

    const next = queryClient.getQueryData<MetaSnapshot>(['snapshot-meta', 'i-1'])

    expect(next?.pendingPermissions?.map((p) => p.requestId)).toEqual(['r-2'])
  })

  it('clears the matching pending row from the permissions store on permission-resolved', async() => {
    const queryClient = buildClient()

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

    await startTranscriptPatcher(queryClient)
    const cb = listeners.get(TauriEvent.AcpPermissionResolved)

    cb!({
      payload: {
        instanceId: 'i-1',
        requestId: 'req-1',
        optionId: 'allow-once'
      }
    })
    await flushPromises()

    expect(usePermissions('i-1').rowQueue.value).toHaveLength(0)
    expect(usePermissions('i-OTHER').rowQueue.value.map((v) => v.request.requestId)).toEqual(['req-cross'])

    cb!({
      payload: {
        instanceId: 'i-OTHER',
        requestId: 'req-cross',
        optionId: 'reject-once'
      }
    })
    await flushPromises()

    expect(usePermissions('i-OTHER').rowQueue.value).toHaveLength(0)
  })

  it('idempotent — second startTranscriptPatcher call returns the existing teardown', async() => {
    const queryClient = buildClient()

    await startTranscriptPatcher(queryClient)
    const firstSize = listeners.size

    await startTranscriptPatcher(queryClient)

    expect(listeners.size).toBe(firstSize)
  })

  it('stamps wire seq onto live-patched items so head.latestSeq stays aligned with daemon', async() => {
    const queryClient = buildClient()

    queryClient.setQueryData(['snapshot-chat', 'i-1'], {
      pages: [chatPage([{ seq: 50, item: { kind: TranscriptItemKind.AgentText, text: 'seed' } as never }])],
      pageParams: [undefined]
    })

    await startTranscriptPatcher(queryClient)
    const cb = listeners.get(TauriEvent.AcpTranscript)

    cb!({
      payload: {
        agentId: 'a',
        instanceId: 'i-1',
        sessionId: 's',
        turnId: 't-1',
        seq: 99,
        item: { kind: TranscriptItemKind.AgentText, text: 'live' }
      } as never
    })
    await flushPromises()

    const page = queryClient.getQueryData<{ pages: ChatSnapshot[] }>(['snapshot-chat', 'i-1'])?.pages[0]

    expect(page?.items).toHaveLength(2)
    expect(page?.items[1].seq).toBe(99)
    expect(page?.latestSeq).toBe(99)
  })

  it('dedups a duplicate live event by wire seq (race-safety on remote reconnect)', async() => {
    const queryClient = buildClient()

    queryClient.setQueryData(['snapshot-chat', 'i-1'], {
      pages: [chatPage([{ seq: 100, item: { kind: TranscriptItemKind.AgentText, text: 'already-applied' } as never }])],
      pageParams: [undefined]
    })

    await startTranscriptPatcher(queryClient)
    const cb = listeners.get(TauriEvent.AcpTranscript)

    cb!({
      payload: {
        agentId: 'a',
        instanceId: 'i-1',
        sessionId: 's',
        turnId: 't-1',
        seq: 100,
        item: { kind: TranscriptItemKind.AgentText, text: 'duplicate' }
      } as never
    })
    await flushPromises()

    const page = queryClient.getQueryData<{ pages: ChatSnapshot[] }>(['snapshot-chat', 'i-1'])?.pages[0]

    expect(page?.items).toHaveLength(1)
    expect((page?.items[0].item as { text: string }).text).toBe('already-applied')
  })

  it('falls back to synthesized seq when wire seq is absent (older daemon)', async() => {
    const queryClient = buildClient()

    queryClient.setQueryData(['snapshot-chat', 'i-1'], {
      pages: [chatPage([{ seq: 7, item: { kind: TranscriptItemKind.AgentText, text: 'seed' } as never }])],
      pageParams: [undefined]
    })

    await startTranscriptPatcher(queryClient)
    const cb = listeners.get(TauriEvent.AcpTranscript)

    cb!({
      payload: {
        agentId: 'a',
        instanceId: 'i-1',
        sessionId: 's',
        item: { kind: TranscriptItemKind.AgentText, text: 'no-seq' }
      } as never
    })
    await flushPromises()

    const page = queryClient.getQueryData<{ pages: ChatSnapshot[] }>(['snapshot-chat', 'i-1'])?.pages[0]

    expect(page?.items).toHaveLength(2)
    expect(page?.items[1].seq).toBe(8)
    expect(page?.latestSeq).toBe(8)
  })
})
