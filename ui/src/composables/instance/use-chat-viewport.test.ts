import { QueryClient, VueQueryPlugin } from '@tanstack/vue-query'
import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { computed, defineComponent, h, ref, type Ref } from 'vue'

import { MAX_PAGES_KEPT, useChatViewport, type UseChatViewportApi } from './use-chat-viewport'
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

  // Live-event patching tests live in `./transcript-patcher.test.ts`
  // since the listener was hoisted to a module-level singleton (see
  // `transcript-patcher.ts`).

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

    api.evictExtraPages()
    await flushPromises()

    // After the trim only the newest MAX_PAGES_KEPT pages remain —
    // the oldest page (seq 50) gets dropped. Items run newest seqs only.
    const seqs = api.items.value.map((it) => it.seq).sort((a, b) => a - b)

    expect(seqs).toHaveLength(MAX_PAGES_KEPT)
    expect(seqs).toContain(200)
    expect(seqs).not.toContain(50)
    unmount()
  })

  it('evictExtraPages is a no-op when cache is at or below MAX_PAGES_KEPT', async() => {
    invoke.mockResolvedValueOnce(chatPage([transcriptText(50, 'p0')], false))
    const id = ref<InstanceId | undefined>('i-1')
    const { api, unmount } = mountViewport(id)

    await flushPromises()
    await flushPromises()

    expect(api.items.value).toHaveLength(1)

    api.evictExtraPages()
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
})

describe('viewportPageSize', () => {
  it('returns the lower-clamp when scrollEl is undefined', async() => {
    const { viewportPageSize } = await import('./use-chat-viewport')
    const elRef = ref<HTMLElement>()

    expect(viewportPageSize(elRef)).toBeGreaterThanOrEqual(20)
  })

  it('returns the lower-clamp when clientHeight is 0 (pre-mount)', async() => {
    const { viewportPageSize } = await import('./use-chat-viewport')
    const fake = { clientHeight: 0 } as HTMLElement
    const elRef = ref<HTMLElement | undefined>(fake)

    expect(viewportPageSize(elRef)).toBeGreaterThanOrEqual(20)
  })

  it('scales with clientHeight past the lower-clamp', async() => {
    const { viewportPageSize } = await import('./use-chat-viewport')

    // Tight viewport — clamps to the floor (20).
    const tight = { clientHeight: 600 } as HTMLElement
    const tightSize = viewportPageSize(ref<HTMLElement | undefined>(tight))

    // Tall (4K-ish) viewport — should exceed the floor.
    const tall = { clientHeight: 4000 } as HTMLElement
    const tallSize = viewportPageSize(ref<HTMLElement | undefined>(tall))

    expect(tightSize).toBeGreaterThanOrEqual(20)
    expect(tallSize).toBeGreaterThan(tightSize)
    // Don't pin exact numbers — the row-height estimate is allowed
    // to drift; just assert the scaling relationship and the floor.
  })
})

describe('measuredPageSize', () => {
  it('falls back to viewportPageSize when no items rendered yet', async() => {
    const { measuredPageSize, viewportPageSize } = await import('./use-chat-viewport')
    const el = { clientHeight: 800, scrollHeight: 0 } as HTMLElement
    const elRef = ref<HTMLElement | undefined>(el)

    expect(measuredPageSize(elRef, 0)).toBe(viewportPageSize(elRef))
  })

  it('returns clientHeight × itemCount / scrollHeight when DOM has items', async() => {
    const { measuredPageSize } = await import('./use-chat-viewport')
    // 30 items rendered into 2400px of content (3 viewports of 800px each).
    // Items-per-viewport = 30 × 800 / 2400 = 10.
    const el = { clientHeight: 800, scrollHeight: 2400 } as HTMLElement

    expect(measuredPageSize(ref<HTMLElement | undefined>(el), 30)).toBe(20)
    // (clamped at MIN_PAGE_SIZE = 20; 10 < 20 so the floor applies)
  })

  it('returns measured value above the floor when content exceeds it', async() => {
    const { measuredPageSize } = await import('./use-chat-viewport')
    // 100 items packed into 2 viewports — average 1 viewport = 50 items.
    const el = { clientHeight: 800, scrollHeight: 1600 } as HTMLElement

    expect(measuredPageSize(ref<HTMLElement | undefined>(el), 100)).toBe(50)
  })

  it('shrinks the next fetch when the viewport contains tall content', async() => {
    const { measuredPageSize } = await import('./use-chat-viewport')
    // 6 items occupy 4800px (avg 800px/item) — 1 viewport (800px) = 1 item.
    const el = { clientHeight: 800, scrollHeight: 4800 } as HTMLElement

    // Measured says 1, floor brings it to 20.
    expect(measuredPageSize(ref<HTMLElement | undefined>(el), 6)).toBe(20)
  })

  it('grows the next fetch when content is dense and short', async() => {
    const { measuredPageSize } = await import('./use-chat-viewport')
    // 200 short items in 1000px — 1 viewport (800px) = 160 items.
    const el = { clientHeight: 800, scrollHeight: 1000 } as HTMLElement

    expect(measuredPageSize(ref<HTMLElement | undefined>(el), 200)).toBe(160)
  })
})
