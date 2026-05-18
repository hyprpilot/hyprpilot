import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { defineComponent, h, ref } from 'vue'

import { useScrollAnchor } from './use-scroll-anchor'

/**
 * Stub `ResizeObserver` — jsdom doesn't ship it. We capture the
 * callback so tests can drive layout-change events directly. One
 * shared mock per file; reset between tests.
 */
let resizeCallbacks: (() => void)[] = []

class StubResizeObserver {
  public callback: ResizeObserverCallback

  constructor(cb: ResizeObserverCallback) {
    this.callback = cb
    resizeCallbacks.push(() => cb([], this as unknown as ResizeObserver))
  }

  public observe(): void {}
  public unobserve(): void {}
  public disconnect(): void {}
}

beforeEach(() => {
  resizeCallbacks = []
  ;(globalThis as unknown as { ResizeObserver: typeof StubResizeObserver }).ResizeObserver = StubResizeObserver
})

afterEach(() => {
  vi.useRealTimers()
})

function fireAllResizes(): void {
  for (const cb of resizeCallbacks) {
    cb()
  }
}

/**
 * Test host that exposes the composable's api refs + the scroll
 * element ref so each test can drive scroll / mutation / resize
 * directly. The DOM shape mirrors what `<Viewport>` renders: a
 * scrollable parent + `<article data-anchor-seq="N">` children.
 */
function buildHost(opts: { stuck: boolean; rows: { seq: number; height: number }[] }) {
  const stuck = ref(opts.stuck)
  let api: ReturnType<typeof useScrollAnchor> | undefined

  const Host = defineComponent({
    setup() {
      const scrollEl = ref<HTMLElement>()

      api = useScrollAnchor(scrollEl, { stuck })

      return () =>
        h(
          'div',
          {
            ref: scrollEl,
            class: 'scroll-root',
            style: 'height: 400px; overflow-y: auto;'
          },
          opts.rows.map((r) =>
            h('article', {
              key: r.seq,
              'data-anchor-seq': String(r.seq),
              style: `height: ${r.height}px;`
            })
          )
        )
    }
  })

  const wrapper = mount(Host, { attachTo: document.body })
  const root = wrapper.find('.scroll-root').element as HTMLElement
  // jsdom doesn't lay out the DOM — fake the geometry the composable
  // reads. We stub `offsetTop` and `offsetHeight` on each row to
  // match the test's declared shape.
  const articles = Array.from(root.querySelectorAll('article')) as HTMLElement[]
  let cursor = 0

  for (let i = 0; i < articles.length; i += 1) {
    const row = opts.rows[i]!
    const el = articles[i]!

    Object.defineProperty(el, 'offsetTop', { configurable: true, value: cursor })
    Object.defineProperty(el, 'offsetHeight', { configurable: true, value: row.height })
    cursor += row.height
  }

  Object.defineProperty(root, 'scrollHeight', { configurable: true, value: cursor })
  Object.defineProperty(root, 'clientHeight', { configurable: true, value: 400 })

  return {
    wrapper,
    root,
    stuck,
    api: api!
  }
}

describe('useScrollAnchor', () => {
  it('captures the topmost visible row on scroll when stuck=false', async() => {
    const { root, api } = buildHost({
      stuck: false,
      rows: [
        { seq: 10, height: 200 },
        { seq: 11, height: 200 },
        { seq: 12, height: 200 }
      ]
    })

    Object.defineProperty(root, 'scrollTop', {
      configurable: true, writable: true, value: 250
    })
    root.dispatchEvent(new Event('scroll'))

    // rAF-throttled; let it fire.
    await new Promise((r) => requestAnimationFrame(r as FrameRequestCallback))
    await flushPromises()

    // scrollTop=250 lands inside row[1] (offsetTop=200, height=200).
    // offsetWithinRow = 250 - 200 = 50.
    expect(api.anchor.value).toEqual({ rowSeq: 11, offsetWithinRow: 50 })
  })

  it('re-locks scrollTop to the anchor row after a resize', async() => {
    const { root, api } = buildHost({
      stuck: false,
      rows: [
        { seq: 10, height: 200 },
        { seq: 11, height: 200 },
        { seq: 12, height: 200 }
      ]
    })

    Object.defineProperty(root, 'scrollTop', {
      configurable: true, writable: true, value: 250
    })
    root.dispatchEvent(new Event('scroll'))
    await new Promise((r) => requestAnimationFrame(r as FrameRequestCallback))
    await flushPromises()

    expect(api.anchor.value?.rowSeq).toBe(11)

    // Simulate row[0] growing by 100px (streaming chunk above the
    // captain's view). offsetTop of row[1] shifts from 200 → 300.
    const row1 = root.querySelectorAll('article')[1] as HTMLElement

    Object.defineProperty(row1, 'offsetTop', { configurable: true, value: 300 })

    // Fire ResizeObserver — anchor's re-lock should set scrollTop to
    // newOffsetTop (300) + offsetWithinRow (50) = 350.
    fireAllResizes()
    await flushPromises()

    expect(root.scrollTop).toBe(350)
  })

  it('does not capture when stuck=true', async() => {
    const { root, api } = buildHost({
      stuck: true,
      rows: [
        { seq: 10, height: 200 },
        { seq: 11, height: 200 }
      ]
    })

    Object.defineProperty(root, 'scrollTop', {
      configurable: true, writable: true, value: 100
    })
    root.dispatchEvent(new Event('scroll'))
    await new Promise((r) => requestAnimationFrame(r as FrameRequestCallback))
    await flushPromises()

    expect(api.anchor.value).toBeUndefined()
  })

  it('drops anchor when row is no longer in DOM (evicted)', async() => {
    const { root, api } = buildHost({
      stuck: false,
      rows: [
        { seq: 10, height: 200 },
        { seq: 11, height: 200 }
      ]
    })

    Object.defineProperty(root, 'scrollTop', {
      configurable: true, writable: true, value: 50
    })
    root.dispatchEvent(new Event('scroll'))
    await new Promise((r) => requestAnimationFrame(r as FrameRequestCallback))
    await flushPromises()

    expect(api.anchor.value?.rowSeq).toBe(10)

    // Remove the anchor row from the DOM. Re-lock should drop the
    // anchor cleanly without writing scrollTop.
    const beforeScrollTop = root.scrollTop
    const row0 = root.querySelectorAll('article')[0]!

    row0.remove()
    fireAllResizes()
    await flushPromises()

    expect(api.anchor.value).toBeUndefined()
    expect(root.scrollTop).toBe(beforeScrollTop)
  })

  it('releaseAnchor clears the anchor synchronously', async() => {
    const { root, api } = buildHost({
      stuck: false,
      rows: [
        { seq: 10, height: 200 },
        { seq: 11, height: 200 }
      ]
    })

    Object.defineProperty(root, 'scrollTop', {
      configurable: true, writable: true, value: 50
    })
    root.dispatchEvent(new Event('scroll'))
    await new Promise((r) => requestAnimationFrame(r as FrameRequestCallback))
    await flushPromises()

    expect(api.anchor.value?.rowSeq).toBe(10)

    api.releaseAnchor()

    expect(api.anchor.value).toBeUndefined()
  })

  it('markProgrammaticScroll suppresses anchor capture for one window', async() => {
    vi.useFakeTimers()
    const { root, api } = buildHost({
      stuck: false,
      rows: [
        { seq: 10, height: 200 },
        { seq: 11, height: 200 }
      ]
    })

    api.markProgrammaticScroll()
    Object.defineProperty(root, 'scrollTop', {
      configurable: true, writable: true, value: 50
    })
    root.dispatchEvent(new Event('scroll'))
    await Promise.resolve()
    // Scroll handler short-circuits without rAF when programmatic is
    // set; anchor stays undefined.
    expect(api.anchor.value).toBeUndefined()

    // After the 500ms clear timer fires, a subsequent scroll captures.
    vi.advanceTimersByTime(500)
    vi.useRealTimers()
    root.dispatchEvent(new Event('scroll'))
    await new Promise((r) => requestAnimationFrame(r as FrameRequestCallback))
    await flushPromises()

    expect(api.anchor.value?.rowSeq).toBe(10)
  })

  it('scrollToSeq finds a row by seq and writes scrollTop', () => {
    const { root, api } = buildHost({
      stuck: false,
      rows: [
        { seq: 10, height: 200 },
        { seq: 11, height: 200 },
        { seq: 12, height: 200 }
      ]
    })

    Object.defineProperty(root, 'scrollTop', {
      configurable: true, writable: true, value: 0
    })

    const found = api.scrollToSeq(11, 25)

    expect(found).toBe(true)
    // row[1].offsetTop = 200, + offsetWithinRow 25 = 225.
    expect(root.scrollTop).toBe(225)
  })

  it('scrollToSeq returns false when seq is not in DOM', () => {
    const { api } = buildHost({
      stuck: false,
      rows: [{ seq: 10, height: 200 }]
    })

    expect(api.scrollToSeq(99)).toBe(false)
  })
})
