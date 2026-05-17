import { mount } from '@vue/test-utils'
import { describe, expect, it, vi } from 'vitest'
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
    value: 1000,
    configurable: true,
    writable: true
  })
  Object.defineProperty(el, 'clientHeight', {
    value: 500,
    configurable: true,
    writable: true
  })
  Object.defineProperty(el, 'scrollTop', {
    value: 500,
    configurable: true,
    writable: true
  })

  const harness: ScrollHarness = {
    el,
    setLayout({ scrollHeight, clientHeight, scrollTop }) {
      Object.defineProperty(el, 'scrollHeight', {
        value: scrollHeight,
        configurable: true,
        writable: true
      })
      Object.defineProperty(el, 'clientHeight', {
        value: clientHeight,
        configurable: true,
        writable: true
      })
      Object.defineProperty(el, 'scrollTop', {
        value: scrollTop,
        configurable: true,
        writable: true
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
      scrollHeight: 1500,
      clientHeight: 500,
      scrollTop: 500
    })
    api.scrollToBottom() // sets scrollTop = scrollHeight = 1500

    // Between the assignment and the scroll event firing, MORE
    // content lands (ToolPill body unfurled).
    harness.setLayout({
      scrollHeight: 1700,
      clientHeight: 500,
      scrollTop: 1500
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
      scrollHeight: 1000,
      clientHeight: 500,
      scrollTop: 100
    })
    harness.dispatchScroll()

    expect(api.stuck.value).toBe(false)
    unmount()
  })

  it('clears the suppress flag on the next scroll event', () => {
    const { api, harness, unmount } = mountHarness()

    // First programmatic scroll — moves scrollTop, sets suppress.
    harness.setLayout({
      scrollHeight: 1500,
      clientHeight: 500,
      scrollTop: 500
    })
    api.scrollToBottom()
    harness.dispatchScroll() // suppress consumed

    // Next user scroll-away should NOT be ignored.
    harness.setLayout({
      scrollHeight: 1500,
      clientHeight: 500,
      scrollTop: 100
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
      scrollHeight: 1000,
      clientHeight: 500,
      scrollTop: 100
    })
    harness.dispatchScroll()

    expect(api.stuck.value).toBe(false)
    unmount()
  })

  /**
   * Chevron click case: the captain has scrolled away (stuck=false),
   * the floating chevron appears, they click it. `scrollToBottom`
   * jumps to the foot and arms the suppress flag. The next scroll
   * event MUST flip `stuck=true` again — without this, the suppress
   * branch swallows the natural restick, the chevron stays
   * permanently visible, and `scheduleStick`'s `!stuck` early-return
   * kills auto-follow on subsequent streaming chunks.
   */
  it('re-establishes stuck=true after a programmatic scroll-to-bottom from a scrolled-away state', () => {
    const { api, harness, unmount } = mountHarness()

    // Captain scrolls up; stuck flips false.
    harness.setLayout({
      scrollHeight: 2000,
      clientHeight: 500,
      scrollTop: 100
    })
    harness.dispatchScroll()
    expect(api.stuck.value).toBe(false)

    // Chevron click → scrollToBottom. Layout now: foot is at 1500,
    // assignment lands there.
    harness.setLayout({
      scrollHeight: 2000,
      clientHeight: 500,
      scrollTop: 100
    })
    api.scrollToBottom()

    // Browser fires the scroll event from the programmatic scroll.
    // The suppress branch must now write stuck=true so the chevron
    // disappears and auto-follow resumes.
    harness.dispatchScroll()
    expect(api.stuck.value).toBe(true)
    unmount()
  })

  /**
   * Captain-reported regression: a small wheel-up during streaming
   * (less than the 64px `nearBottom` threshold) didn't flip
   * `stuck=false`, so the next `scheduleStick` pass from a
   * MutationObserver chunk snapped the captain back to the foot.
   * Equivalent surface: dragging the OS scrollbar a few pixels up —
   * the native scrollbar widget fires `scroll` events but no
   * `pointerdown` / `wheel`, so without direction-based detection
   * this would also be invisible to the gate.
   * Direction-based detection (any decreasing `scrollTop` in a
   * non-suppressed scroll event) catches both.
   */
  it('flips stuck=false on a tiny upward scroll well below the 64px threshold', () => {
    const { api, harness, unmount } = mountHarness()

    // Establish the at-bottom baseline. Default layout already has
    // scrollTop=500 of a 1000-scrollHeight / 500-clientHeight box
    // (distance from bottom = 0). The first scroll event seeds
    // `prevScrollTop` to that value.
    expect(api.stuck.value).toBe(true)
    harness.dispatchScroll()
    expect(api.stuck.value).toBe(true)

    // Captain nudges the wheel up by 20px — well within the 64px
    // threshold. `nearBottom` would still return true, but the
    // movement is upward, so stuck flips false immediately.
    harness.setLayout({
      scrollHeight: 1000,
      clientHeight: 500,
      scrollTop: 480
    })
    harness.dispatchScroll()

    expect(api.stuck.value).toBe(false)
    unmount()
  })

  /**
   * Captain-reported regression: while stuck=true, PageUp / mouse
   * wheel / OS scrollbar did nothing. Cause: any text mutation in
   * the viewport's subtree (a StreamCard elapsed chip ticking every
   * second, a tool-call status flipping) fires the MutationObserver
   * → `scheduleStick` → rAF queued. If the captain's upward gesture
   * lands inside the ~16ms schedule→fire window, the rAF still
   * runs `scrollToBottom` (by design — re-checking `stuck.value`
   * at fire time caused a different bug). The fix is to cancel
   * the pending rAF from `onScroll` when movement is upward —
   * direction is the captain's intent signal.
   */
  it('cancels a pending scheduleStick rAF when the captain scrolls up before it fires', async() => {
    const rafCallbacks = new Map<number, FrameRequestCallback>()
    let rafIdSeq = 0
    const rafSpy = vi.fn((cb: FrameRequestCallback) => {
      const id = ++rafIdSeq

      rafCallbacks.set(id, cb)

      return id
    })
    const cancelSpy = vi.fn((id: number) => {
      rafCallbacks.delete(id)
    })

    vi.stubGlobal('requestAnimationFrame', rafSpy)
    vi.stubGlobal('cancelAnimationFrame', cancelSpy)

    try {
      const { api, harness, unmount } = mountHarness()

      // Seed `prevScrollTop` to the at-bottom baseline (500) so the
      // upward dispatch below registers as a real upward gesture.
      harness.dispatchScroll()
      expect(api.stuck.value).toBe(true)

      // Trigger MutationObserver → scheduleStick. jsdom delivers
      // MutationObserver records on a microtask; await one tick.
      harness.el.appendChild(document.createElement('div'))
      await Promise.resolve()

      // rAF was queued by scheduleStick.
      expect(rafSpy).toHaveBeenCalled()
      expect(rafCallbacks.size).toBe(1)

      // Captain wheels up before the rAF fires.
      harness.setLayout({
        scrollHeight: 2000,
        clientHeight: 500,
        scrollTop: 100
      })
      harness.dispatchScroll()

      expect(api.stuck.value).toBe(false)
      // The pending rAF was cancelled — without the cancel, it would
      // fire on the next frame and snap the captain back to the foot.
      expect(cancelSpy).toHaveBeenCalled()
      expect(rafCallbacks.size).toBe(0)
      unmount()
    } finally {
      vi.unstubAllGlobals()
    }
  })

  /**
   * Downward (or no) movement at-or-near the bottom should NOT
   * spuriously unstick — the existing threshold check still
   * governs that direction. A following live tail with brief
   * overshoot (small jitter past the foot) must keep `stuck=true`.
   */
  it('stays stuck on downward / unchanged scroll within the threshold', () => {
    const { api, harness, unmount } = mountHarness()

    // Seed baseline.
    harness.dispatchScroll()
    expect(api.stuck.value).toBe(true)

    // scrollTop unchanged — still at bottom. nearBottom returns
    // true, no upward movement. Stuck stays true.
    harness.dispatchScroll()
    expect(api.stuck.value).toBe(true)

    // scrollTop moved down a few px (e.g., browser-internal jitter
    // during a chunk land). Still within threshold of the foot.
    harness.setLayout({
      scrollHeight: 1010,
      clientHeight: 500,
      scrollTop: 510
    })
    harness.dispatchScroll()
    expect(api.stuck.value).toBe(true)
    unmount()
  })
})
