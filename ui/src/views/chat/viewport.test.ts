import { QueryClient, VueQueryPlugin } from '@tanstack/vue-query'
import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import Viewport from './Viewport.vue'
import { useActiveInstance } from '@composables'
import { TranscriptItemKind, type ChatSnapshot, type SeqTranscriptItem } from '@ipc'

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

// `@tanstack/vue-virtual`'s `useVirtualizer` reaches for live layout
// (offsetHeight / scroll metrics) that JSDOM doesn't compute. We
// stub the helper to return a tiny synchronous virtualizer that
// renders all rows — Viewport.vue's template binds to
// `getVirtualItems()` and `getTotalSize()`, and we want every block
// in the DOM so the assertions can find them by data-attr.
//
// Imports `computed` lazily inside the factory because vi.mock
// runs before module evaluation; top-of-file imports aren't
// available at this point.
vi.mock('@tanstack/vue-virtual', async() => {
  const { computed } = await import('vue')

  return {
    useVirtualizer: (optsRef: unknown) => {
      const readCount = (): number => {
        const raw = (optsRef as { value?: unknown }).value !== undefined ? (optsRef as { value: { count?: number } }).value : (optsRef as { count?: number })

        return raw?.count ?? 0
      }

      // Return a Ref-like wrapper with a stable `.value` shape so
      // `virtualizer.value.getVirtualItems()` works in the template.
      // The stub re-reads `count` each time `getVirtualItems` runs
      // so reactive blocks-length flips are picked up.
      return computed(() => ({
        getVirtualItems: () => {
          const count = readCount()

          return Array.from({ length: count }, (_, i) => ({
            index: i,
            key: i,
            start: i * 160,
            size: 160
          }))
        },
        getTotalSize: () => readCount() * 160,
        measureElement: () => undefined,
        scrollToIndex: () => undefined
      }))
    }
  }
})

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

function chatPage(items: SeqTranscriptItem[], hasMore = false): ChatSnapshot {
  return {
    items,
    oldestSeq: items[0]?.seq,
    latestSeq: items[items.length - 1]?.seq,
    hasMore
  }
}

function seedQueryData(qc: QueryClient, instanceId: string, page: ChatSnapshot): void {
  qc.setQueryData(['snapshot-chat', instanceId], { pages: [page], pageParams: [undefined] })
}

interface MountOpts {
  instanceId: string
  seedPage?: ChatSnapshot
  /// Override the resolve of the first invoke call (the initial fetch).
  initialPage?: ChatSnapshot
}

function mountViewport(opts: MountOpts) {
  if (opts.initialPage !== undefined) {
    invoke.mockResolvedValueOnce(opts.initialPage)
  }
  const qc = buildClient()

  if (opts.seedPage !== undefined) {
    seedQueryData(qc, opts.instanceId, opts.seedPage)
  }

  // Fix the focused instance id BEFORE mount so the composable's
  // `enabled` flag flips on at first render.
  useActiveInstance().id.value = opts.instanceId

  return mount(Viewport, {
    global: {
      plugins: [[VueQueryPlugin, { queryClient: qc }]],
      stubs: {
        // Heavy children we don't need to assert on for this test —
        // their own tests cover their internals.
        Turn: { template: '<div class="stub-turn" data-stub="turn"><slot /></div>' },
        StreamCard: { template: '<div class="stub-stream-card" />' },
        ChangeBanner: { template: '<div class="stub-change-banner" />' },
        ToolChips: { template: '<div class="stub-tool-chips" />' },
        TerminalCard: { template: '<div class="stub-terminal-card" />' },
        Body: { template: '<div class="stub-chat-body" data-stub="chat-body"><slot /></div>' },
        Attachments: { template: '<div class="stub-attachments" />' }
      }
    }
  })
}

// jsdom doesn't ship `IntersectionObserver`. `@vueuse/core`'s
// `useIntersectionObserver` no-ops when it's missing. Stub the global
// with a capture-shim so individual tests can manually invoke the
// callback the composable registered, simulating the sentinel
// entering the viewport without a real layout system.
const intersectionCallbacks = new Set<IntersectionObserverCallback>()

class IntersectionObserverStub implements IntersectionObserver {
  public readonly root: Element | Document | null = null

  public readonly rootMargin: string = ''

  public readonly scrollMargin: string = ''

  public readonly thresholds: readonly number[] = []

  constructor(public callback: IntersectionObserverCallback) {
    intersectionCallbacks.add(callback)
  }

  public observe(): void {}

  public unobserve(): void {}

  public disconnect(): void {
    intersectionCallbacks.delete(this.callback)
  }

  public takeRecords(): IntersectionObserverEntry[] {
    return []
  }
}

vi.stubGlobal('IntersectionObserver', IntersectionObserverStub)

function fireIntersection(isIntersecting: boolean): void {
  for (const cb of intersectionCallbacks) {
    cb(
      [
        {
          isIntersecting,
          intersectionRatio: isIntersecting ? 1 : 0,
          target: document.createElement('div'),
          time: performance.now()
        } as unknown as IntersectionObserverEntry
      ],
      {} as IntersectionObserver
    )
  }
}

beforeEach(() => {
  invoke.mockReset()
  listeners.clear()
  intersectionCallbacks.clear()
  useActiveInstance().id.value = undefined
})

describe('Viewport.vue', () => {
  it('renders a virtual row per cached snapshot block', async() => {
    const page = chatPage(
      [
        {
          seq: 1,
          item: {
            kind: TranscriptItemKind.UserPrompt,
            text: 'hi',
            attachments: []
          } as never
        },
        {
          seq: 2,
          item: { kind: TranscriptItemKind.AgentText, text: 'hello' } as never
        }
      ],
      false
    )
    const wrapper = mountViewport({ instanceId: 'i-1', initialPage: page })

    // Two flushes: one for the queryFn promise, one for the
    // post-fetch reactive update + virtual-host re-render.
    await flushPromises()
    await flushPromises()
    await flushPromises()
    await wrapper.vm.$nextTick()

    // One virtual row per timeline block — user prompt + agent reply
    // collapse into two blocks (user + assistant).
    const rows = wrapper.findAll('[data-stub="turn"]')

    expect(rows.length).toBe(2)
    wrapper.unmount()
  })

  it('renders the empty slot when the snapshot has no items', async() => {
    invoke.mockResolvedValueOnce(chatPage([], false))
    useActiveInstance().id.value = 'i-empty'
    const wrapper = mount(Viewport, {
      slots: { empty: '<div data-testid="empty-slot">no chat yet</div>' },
      global: {
        plugins: [[VueQueryPlugin, { queryClient: buildClient() }]],
        stubs: {
          Turn: { template: '<div />' },
          StreamCard: { template: '<div />' },
          ChangeBanner: { template: '<div />' },
          ToolChips: { template: '<div />' },
          TerminalCard: { template: '<div />' },
          Body: { template: '<div />' },
          Attachments: { template: '<div />' }
        }
      }
    })

    await flushPromises()
    await flushPromises()
    expect(wrapper.find('[data-testid="empty-slot"]').exists()).toBe(true)
    wrapper.unmount()
  })

  it('hides the load chip when hasNextPage is false', async() => {
    const exhausted = chatPage(
      [
        {
          seq: 5,
          item: { kind: TranscriptItemKind.AgentText, text: 'all there is' } as never
        }
      ],
      false
    )

    invoke.mockResolvedValueOnce(exhausted)
    const wrapper = mountViewport({ instanceId: 'i-1', initialPage: exhausted })

    await flushPromises()
    await flushPromises()
    // Chip only renders while `isFetchingNextPage` is true; with
    // `hasMore: false` the next fetch never fires, so the chip stays
    // hidden through the lifecycle.
    expect(wrapper.find('[data-testid="chat-load-chip"]').exists()).toBe(false)
    wrapper.unmount()
  })

  it('renders the top sentinel when older pages are available so IntersectionObserver can drive backward fetch', async() => {
    // PR4 refactor: the scrollTop-threshold trigger is gone. Backward
    // fetch is wired via `useIntersectionObserver` against a top
    // sentinel that only renders when `hasNextPage` is true. jsdom's
    // IntersectionObserver is a no-op stub, so we verify the wiring
    // shape — sentinel present + `hasNextPage` true — instead of
    // simulating the observer callback.
    const first = chatPage(
      [
        {
          seq: 100,
          item: { kind: TranscriptItemKind.AgentText, text: 'p0' } as never
        }
      ],
      true
    )

    invoke.mockResolvedValueOnce(first)
    const wrapper = mountViewport({ instanceId: 'i-1', initialPage: first })

    await flushPromises()
    await flushPromises()
    await wrapper.vm.$nextTick()

    const sentinel = wrapper.find('[data-testid="chat-top-sentinel"]')

    expect(sentinel.exists()).toBe(true)
    wrapper.unmount()
  })

  it('manually firing the intersection observer triggers fetchNextPage with the head page cursor', async() => {
    const first = chatPage(
      [
        {
          seq: 100,
          item: { kind: TranscriptItemKind.AgentText, text: 'p0' } as never
        }
      ],
      true
    )

    invoke.mockResolvedValueOnce(first)
    const wrapper = mountViewport({ instanceId: 'i-1', initialPage: first })

    await flushPromises()
    await flushPromises()
    await wrapper.vm.$nextTick()

    invoke.mockClear()
    invoke.mockResolvedValueOnce(chatPage([], false))

    // Simulate the sentinel entering the viewport. The observer
    // callback runs `viewport.fetchNextPage()` which invokes
    // `instance_snapshot_chat` with `before = oldestSeq` of the
    // current head page.
    fireIntersection(true)
    await flushPromises()
    await flushPromises()

    expect(invoke).toHaveBeenCalled()
    const lastCall = invoke.mock.calls[invoke.mock.calls.length - 1]

    expect(lastCall?.[0]).toBe('instance_snapshot_chat')
    expect((lastCall?.[1] as { before?: number })?.before).toBe(100)
    wrapper.unmount()
  })

  it('omits the top sentinel when no older pages remain (hasNextPage = false)', async() => {
    const onlyPage = chatPage(
      [
        {
          seq: 0,
          item: { kind: TranscriptItemKind.AgentText, text: 'first-ever' } as never
        }
      ],
      false
    )

    invoke.mockResolvedValueOnce(onlyPage)
    const wrapper = mountViewport({ instanceId: 'i-1', initialPage: onlyPage })

    await flushPromises()
    await flushPromises()
    await wrapper.vm.$nextTick()

    const sentinel = wrapper.find('[data-testid="chat-top-sentinel"]')

    expect(sentinel.exists()).toBe(false)
    wrapper.unmount()
  })

  it('PageDown scrolls the transcript by ~90% of the viewport height', async() => {
    const page = chatPage(
      [
        {
          seq: 1,
          item: { kind: TranscriptItemKind.AgentText, text: 'page' } as never
        }
      ],
      false
    )

    invoke.mockResolvedValueOnce(page)
    const wrapper = mountViewport({ instanceId: 'i-1', initialPage: page })

    await flushPromises()
    await flushPromises()

    const root = wrapper.find('[data-testid="chat-transcript"]').element as HTMLElement
    const scrollBy = vi.fn()

    Object.defineProperty(root, 'scrollBy', { configurable: true, value: scrollBy })
    Object.defineProperty(root, 'clientHeight', { configurable: true, value: 400 })

    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'PageDown', bubbles: true }))
    await flushPromises()

    expect(scrollBy).toHaveBeenCalledWith(expect.objectContaining({ top: 360 }))

    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'PageUp', bubbles: true }))
    await flushPromises()

    expect(scrollBy).toHaveBeenLastCalledWith(expect.objectContaining({ top: -360 }))

    wrapper.unmount()
  })

  it('PageDown still scrolls when focus is in an input / textarea', async() => {
    // The captain's focus lives in the composer textarea ~99% of the
    // time, so an `isEditableTarget` bailout meant Page keys were
    // effectively dead. We bypass the bailout for PageUp/PageDown
    // specifically — they're unambiguously about scrolling a big
    // container, and textareas have no meaningful pageful navigation.
    // Home/End keep the bailout (see next test) because they DO have
    // meaningful in-textarea behaviour.
    const page = chatPage(
      [
        {
          seq: 1,
          item: { kind: TranscriptItemKind.AgentText, text: 'page' } as never
        }
      ],
      false
    )

    invoke.mockResolvedValueOnce(page)
    const wrapper = mountViewport({ instanceId: 'i-1', initialPage: page })

    await flushPromises()
    await flushPromises()

    const root = wrapper.find('[data-testid="chat-transcript"]').element as HTMLElement
    const scrollBy = vi.fn()

    Object.defineProperty(root, 'scrollBy', { configurable: true, value: scrollBy })
    Object.defineProperty(root, 'clientHeight', { configurable: true, value: 400 })

    const ta = document.createElement('textarea')

    document.body.appendChild(ta)
    ta.focus()
    ta.dispatchEvent(new KeyboardEvent('keydown', { key: 'PageDown', bubbles: true }))
    await flushPromises()

    expect(scrollBy).toHaveBeenCalledWith(expect.objectContaining({ top: 360 }))

    ta.dispatchEvent(new KeyboardEvent('keydown', { key: 'PageUp', bubbles: true }))
    await flushPromises()

    expect(scrollBy).toHaveBeenLastCalledWith(expect.objectContaining({ top: -360 }))

    document.body.removeChild(ta)
    wrapper.unmount()
  })

  it('Home / End also bypass the editable-target gate (captain wants viewport nav)', async() => {
    // Home and End used to bail when focus was in the composer
    // textarea so they could still navigate the caret. Captain
    // pivoted: their focus lives in the composer 99% of the time
    // and mid-line caret hops are rare — viewport scroll is the
    // dominant intent. Match PageUp/PageDown's behavior: bypass
    // the gate, drive viewport scroll, leave caret nav to
    // Ctrl+Home / Ctrl+End at the OS level.
    const page = chatPage(
      [
        {
          seq: 1,
          item: { kind: TranscriptItemKind.AgentText, text: 'page' } as never
        }
      ],
      false
    )

    invoke.mockResolvedValueOnce(page)
    const wrapper = mountViewport({ instanceId: 'i-1', initialPage: page })

    await flushPromises()
    await flushPromises()

    const root = wrapper.find('[data-testid="chat-transcript"]').element as HTMLElement
    const scrollTo = vi.fn()

    Object.defineProperty(root, 'scrollTo', { configurable: true, value: scrollTo })

    const ta = document.createElement('textarea')

    document.body.appendChild(ta)
    ta.focus()
    ta.dispatchEvent(new KeyboardEvent('keydown', { key: 'Home', bubbles: true }))
    ta.dispatchEvent(new KeyboardEvent('keydown', { key: 'End', bubbles: true }))
    await flushPromises()

    expect(scrollTo).toHaveBeenCalledTimes(2)
    expect(scrollTo).toHaveBeenCalledWith({ top: 0, behavior: 'smooth' })

    document.body.removeChild(ta)
    wrapper.unmount()
  })

  it('does not refetch on scroll when there are no older pages', async() => {
    const exhausted = chatPage(
      [
        {
          seq: 5,
          item: { kind: TranscriptItemKind.AgentText, text: 'a' } as never
        }
      ],
      false
    )

    invoke.mockResolvedValueOnce(exhausted)
    const wrapper = mountViewport({ instanceId: 'i-1', initialPage: exhausted })

    await flushPromises()
    await flushPromises()

    invoke.mockClear()

    const root = wrapper.find('[data-testid="chat-transcript"]').element as HTMLElement

    Object.defineProperty(root, 'scrollTop', { configurable: true, value: 0 })
    root.dispatchEvent(new Event('scroll'))

    await flushPromises()

    expect(invoke).not.toHaveBeenCalled()
    wrapper.unmount()
  })

  it('defers stuck eviction to requestAnimationFrame', async() => {
    // Eviction inside the scroll-event task removes DOM nodes from
    // the TOP of the scroller while the browser is mid-gesture,
    // confusing concurrent stick-to-bottom observers. The body view
    // wraps the trigger in rAF so the cache mutation lands on the
    // next frame, after the browser has finished the current scroll
    // tick. The current gate fires when `stuck=true` (captain at the
    // foot, foot-follow path owns scroll) AND `hasUserScrolled=true`
    // (captain has actually engaged — distinguishing from mount-time
    // synthetic scrolls).
    const page = chatPage(
      [
        {
          seq: 1,
          item: { kind: TranscriptItemKind.AgentText, text: 'a' } as never
        }
      ],
      false
    )

    invoke.mockResolvedValueOnce(page)
    const wrapper = mountViewport({ instanceId: 'i-1', initialPage: page })

    await flushPromises()
    await flushPromises()

    const root = wrapper.find('[data-testid="chat-transcript"]').element as HTMLElement

    // `useStickToBottom`'s `stuck` starts true. We want to keep it
    // true through the rest of the test — wheel-up would flip it to
    // false via `releaseStickAndAnchor`. Use `pointerdown` to set
    // `hasUserScrolled` without releasing stick.
    root.dispatchEvent(new PointerEvent('pointerdown'))
    await flushPromises()

    // Position the captain at the foot — `useStickToBottom`'s next
    // `onScroll` sees `nearBottom=true` and keeps `stuck=true`.
    Object.defineProperty(root, 'scrollHeight', {
      configurable: true,
      value: 2000
    })
    Object.defineProperty(root, 'clientHeight', {
      configurable: true,
      value: 800
    })
    Object.defineProperty(root, 'scrollTop', {
      configurable: true,
      writable: true,
      value: 1200
    })
    const rafSpy = vi.spyOn(window, 'requestAnimationFrame')

    root.dispatchEvent(new Event('scroll'))

    expect(rafSpy).toHaveBeenCalled()

    rafSpy.mockRestore()
    wrapper.unmount()
  })
})
