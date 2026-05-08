import { QueryClient, VueQueryPlugin } from '@tanstack/vue-query'
import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { defineComponent, h, ref, type Ref } from 'vue'

import { DEFAULT_CHAT_LIMIT, useInstanceChatInfiniteQuery } from './use-instance-chat-infinite-query'
import { TauriCommand, type ChatSnapshot, type SeqTranscriptItem } from '@ipc'

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }))

vi.mock('@ipc/bridge', async() => ({
  ...(await vi.importActual<object>('@ipc/bridge')),
  invoke: (command: string, args?: Record<string, unknown>) => invoke(command, args),
  listen: vi.fn()
}))

function transcript(seq: number): SeqTranscriptItem {
  return {
    seq,
    item: { kind: 'agent_text', text: `chunk-${seq}` } as never
  }
}

function buildClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: {
        retry: false,
        gcTime: 0,
        staleTime: 0
      }
    }
  })
}

interface Probe {
  data: Ref<{ pages: ChatSnapshot[]; pageParams: unknown[] } | undefined>
  isPending: Ref<boolean>
  isError: Ref<boolean>
  error: Ref<Error | null>
  hasNextPage: Ref<boolean>
  hasPreviousPage: Ref<boolean>
  fetchNextPage: () => Promise<unknown>
}

function mountWith(idRef: Ref<string | undefined>): { probe: Probe; unmount: () => void } {
  const probe: Partial<Probe> = {}
  const TestComponent = defineComponent({
    setup() {
      const id = ref(idRef.value)

      Object.assign(probe, useInstanceChatInfiniteQuery(id as unknown as Ref<string | undefined> as never))

      return () => h('div')
    }
  })
  const queryClient = buildClient()
  const wrapper = mount(TestComponent, {
    global: { plugins: [[VueQueryPlugin, { queryClient }]] }
  })

  return { probe: probe as Probe, unmount: () => wrapper.unmount() }
}

beforeEach(() => {
  invoke.mockReset()
})

describe('useInstanceChatInfiniteQuery', () => {
  it('first page calls invoke with before=undefined', async() => {
    const page: ChatSnapshot = {
      items: [transcript(150), transcript(151)],
      oldestSeq: 150,
      latestSeq: 151,
      hasMore: true
    }

    invoke.mockResolvedValueOnce(page)
    const id = ref<string | undefined>('i-1')
    const { probe, unmount } = mountWith(id)

    await flushPromises()
    await flushPromises()

    expect(invoke).toHaveBeenCalledTimes(1)
    expect(invoke).toHaveBeenCalledWith(TauriCommand.InstanceSnapshotChat, {
      instanceId: 'i-1',
      before: undefined,
      limit: DEFAULT_CHAT_LIMIT
    })
    expect(probe.hasNextPage.value).toBe(true)
    expect(probe.hasPreviousPage.value).toBe(false)
    unmount()
  })

  it('fetchNextPage passes before=oldestSeq from the last page', async() => {
    const first: ChatSnapshot = {
      items: [transcript(150)],
      oldestSeq: 150,
      latestSeq: 150,
      hasMore: true
    }
    const second: ChatSnapshot = {
      items: [transcript(100)],
      oldestSeq: 100,
      latestSeq: 100,
      hasMore: false
    }

    invoke.mockResolvedValueOnce(first).mockResolvedValueOnce(second)
    const id = ref<string | undefined>('i-1')
    const { probe, unmount } = mountWith(id)

    await flushPromises()
    await flushPromises()

    await probe.fetchNextPage()
    await flushPromises()
    await flushPromises()

    expect(invoke).toHaveBeenCalledTimes(2)
    expect(invoke).toHaveBeenLastCalledWith(TauriCommand.InstanceSnapshotChat, {
      instanceId: 'i-1',
      before: 150,
      limit: DEFAULT_CHAT_LIMIT
    })
    // Second page reported `hasMore = false` so `hasNextPage` flips off.
    expect(probe.hasNextPage.value).toBe(false)
    unmount()
  })

  it('hasNextPage reflects hasMore on the latest fetched page', async() => {
    const exhausted: ChatSnapshot = {
      items: [transcript(0)],
      oldestSeq: 0,
      latestSeq: 0,
      hasMore: false
    }

    invoke.mockResolvedValueOnce(exhausted)
    const id = ref<string | undefined>('i-1')
    const { probe, unmount } = mountWith(id)

    await flushPromises()
    await flushPromises()

    expect(probe.hasNextPage.value).toBe(false)
    unmount()
  })

  it('is disabled when instanceId is undefined', async() => {
    invoke.mockResolvedValue({ items: [], hasMore: false })
    const id = ref<string | undefined>(undefined)
    const { probe, unmount } = mountWith(id)

    await flushPromises()
    await flushPromises()

    expect(invoke).not.toHaveBeenCalled()
    expect(probe.data.value).toBeUndefined()
    unmount()
  })
})
