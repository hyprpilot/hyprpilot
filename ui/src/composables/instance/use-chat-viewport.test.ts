import { QueryClient, VueQueryPlugin } from '@tanstack/vue-query'
import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { computed, defineComponent, h, ref, type Ref } from 'vue'

import { useChatViewport, type UseChatViewportApi } from './use-chat-viewport'
import type { InstanceId } from '../chrome/use-active-instance'
import { TranscriptItemKind, type ChatSnapshot, type SeqTranscriptItem } from '@ipc'

const { invoke } = vi.hoisted(() => ({
  invoke: vi.fn()
}))

vi.mock('@ipc', async() => ({
  ...(await vi.importActual<object>('@ipc')),
  invoke: (command: string, args?: Record<string, unknown>) => invoke(command, args)
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
    unmount()
  })

  it('keeps every cached page without client-side trimming', async() => {
    const id = ref<InstanceId | undefined>('i-1')
    const { api, queryClient, unmount } = mountViewport(id)

    queryClient.setQueryData(['snapshot-chat', 'i-1'], {
      pages: [
        chatPage([transcriptText(200, 'p0')], false),
        chatPage([transcriptText(150, 'p1')], false),
        chatPage([transcriptText(100, 'p2')], false),
        chatPage([transcriptText(50, 'p3')], false)
      ],
      pageParams: [undefined, 200, 150, 100]
    })
    await flushPromises()

    expect(api.items.value.map((it) => it.seq)).toEqual([50, 100, 150, 200])
    unmount()
  })

  it('deduplicates overlapping seqs defensively', async() => {
    const id = ref<InstanceId | undefined>('i-1')
    const { api, queryClient, unmount } = mountViewport(id)

    queryClient.setQueryData(['snapshot-chat', 'i-1'], {
      pages: [chatPage([transcriptText(2, 'new')], false), chatPage([transcriptText(1, 'old'), transcriptText(2, 'duplicate')], false)],
      pageParams: [undefined, 2]
    })
    await flushPromises()

    expect(api.items.value.map((it) => it.seq)).toEqual([1, 2])
    unmount()
  })
})
