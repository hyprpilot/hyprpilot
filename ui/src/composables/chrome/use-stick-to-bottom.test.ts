import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'
import { defineComponent, h, ref, type Ref } from 'vue'

import { useStickToBottom } from './use-stick-to-bottom'

interface ScrollHarness {
  el: HTMLElement
  setLayout: (opts: { scrollHeight: number; clientHeight: number; scrollTop: number }) => void
  dispatchScroll: () => void
}

/**
 * Mount a component that exposes the useStickToBottom api against a
 * real DOM <div>, with overridden `scrollHeight` / `clientHeight` /
 * `scrollTop` so we can simulate layout state. The MutationObserver
 * needs a real Node to observe; the assignments here just give it a
 * surface — we assert behavior off the api refs, not the DOM render.
 */
function mountHarness(): { api: { stuck: Ref<boolean>; scrollToBottom: () => void }; harness: ScrollHarness; unmount: () => void } {
  let api: { stuck: Ref<boolean>; scrollToBottom: () => void } | undefined

  const Comp = defineComponent({
    setup() {
      const elRef = ref<HTMLElement | undefined>(undefined)

      api = useStickToBottom(elRef)

      return () => h('div', { ref: elRef }, [])
    }
  })

  const wrapper = mount(Comp)
  const el = wrapper.element as HTMLElement

  // Default layout: tiny container, parked at bottom.
  Object.defineProperty(el, 'scrollHeight', {
    value: 1000, configurable: true, writable: true
  })
  Object.defineProperty(el, 'clientHeight', {
    value: 500, configurable: true, writable: true
  })
  Object.defineProperty(el, 'scrollTop', {
    value: 500, configurable: true, writable: true
  })

  const harness: ScrollHarness = {
    el,
    setLayout({ scrollHeight, clientHeight, scrollTop }) {
      Object.defineProperty(el, 'scrollHeight', {
        value: scrollHeight, configurable: true, writable: true
      })
      Object.defineProperty(el, 'clientHeight', {
        value: clientHeight, configurable: true, writable: true
      })
      Object.defineProperty(el, 'scrollTop', {
        value: scrollTop, configurable: true, writable: true
      })
    },
    dispatchScroll() {
      el.dispatchEvent(new Event('scroll'))
    }
  }

  return {
    api: api!,
    harness,
    unmount: () => wrapper.unmount()
  }
}

describe('useStickToBottom', () => {
  it('stays stuck when content growth fires a scroll event AFTER programmatic scrollToBottom', () => {
    const { api, harness, unmount } = mountHarness()

    // Start at bottom: scrollTop=500 = scrollHeight(1000) - clientHeight(500).
    expect(api.stuck.value).toBe(true)

    // Streaming update lands new content. Mutation observer would fire
    // scheduleStick → scrollToBottom. The browser fires a scroll event
    // for the programmatic scroll AFTER content has grown further
    // (e.g. ToolPill auto-expanding inline).
    harness.setLayout({
      scrollHeight: 1500, clientHeight: 500, scrollTop: 500
    })
    api.scrollToBottom() // sets scrollTop = scrollHeight = 1500

    // Between the assignment and the scroll event firing, MORE
    // content lands (ToolPill body unfurled).
    harness.setLayout({
      scrollHeight: 1700, clientHeight: 500, scrollTop: 1500
    })

    // NOW the scroll event from the programmatic scroll fires.
    // nearBottom would compute 1700 - 1500 - 500 = -300; clamped, but
    // the gap (200px) exceeds threshold(64). Without the suppress
    // flag, stuck would flip false. With it, stays true.
    harness.dispatchScroll()

    expect(api.stuck.value).toBe(true)
    unmount()
  })

  it('flips stuck=false on a real user scroll away from bottom', () => {
    const { api, harness, unmount } = mountHarness()

    expect(api.stuck.value).toBe(true)

    // User scrolls up to the middle.
    harness.setLayout({
      scrollHeight: 1000, clientHeight: 500, scrollTop: 100
    })
    harness.dispatchScroll()

    expect(api.stuck.value).toBe(false)
    unmount()
  })

  it('clears the suppress flag on the next scroll event', () => {
    const { api, harness, unmount } = mountHarness()

    // First programmatic scroll — moves scrollTop, sets suppress.
    harness.setLayout({
      scrollHeight: 1500, clientHeight: 500, scrollTop: 500
    })
    api.scrollToBottom()
    harness.dispatchScroll() // suppress consumed

    // Next user scroll-away should NOT be ignored.
    harness.setLayout({
      scrollHeight: 1500, clientHeight: 500, scrollTop: 100
    })
    harness.dispatchScroll()

    expect(api.stuck.value).toBe(false)
    unmount()
  })

  it('does not arm the suppress flag when scrollToBottom is a no-op', () => {
    const { api, harness, unmount } = mountHarness()

    // Already at bottom. scrollToBottom assignment is a no-op
    // (browser would not fire a scroll event for an unchanged value).
    api.scrollToBottom()

    // A user scroll away should NOT be swallowed by a stale flag.
    harness.setLayout({
      scrollHeight: 1000, clientHeight: 500, scrollTop: 100
    })
    harness.dispatchScroll()

    expect(api.stuck.value).toBe(false)
    unmount()
  })
})
