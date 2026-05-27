import { QueryClient, VueQueryPlugin } from '@tanstack/vue-query'
import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import Viewport from './Viewport.vue'
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

  return mount(Viewport, {
    props: {
      instanceId: opts.instanceId,
      active: true
    },
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

beforeEach(() => {
  invoke.mockReset()
  listeners.clear()
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
    const wrapper = mount(Viewport, {
      props: { instanceId: 'i-empty', active: true },
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

  it('marks inactive retained viewports as hidden at the root', async() => {
    invoke.mockResolvedValueOnce(chatPage([], false))
    const wrapper = mount(Viewport, {
      props: { instanceId: 'i-hidden', active: false },
      slots: { empty: '<div />' },
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

    expect(wrapper.find('.chat-viewport-root').attributes('data-active')).toBe('false')
    wrapper.unmount()
  })

  it('does not refetch on scroll because lazy loading is disabled', async() => {
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
})
