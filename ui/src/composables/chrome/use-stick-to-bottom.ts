import { onMounted, onUnmounted, ref, type Ref } from 'vue'

/**
 * Auto-scroll behavior for a long-running feed (the chat transcript).
 * Sticks to the bottom while the user is already there; pauses
 * sticking the moment the user scrolls up so reading older messages
 * isn't yanked out from under them. Resumes when they scroll back to
 * within `threshold` px of the bottom.
 *
 * Two observers do the work:
 *   - `MutationObserver` catches new children + text edits (every
 *     transcript chunk lands as a DOM mutation).
 *   - `ResizeObserver` catches reflows inside existing children
 *     (long-running tool output growing inline, code blocks
 *     expanding, etc.) which `MutationObserver` doesn't.
 *
 * `stuck` is exposed for callers that want a "scroll to bottom"
 * affordance — `false` means the user has scrolled away.
 */
export function useStickToBottom(scrollEl: Ref<HTMLElement | undefined>, options?: { threshold?: number }): { stuck: Ref<boolean>; scrollToBottom: () => void } {
  const threshold = options?.threshold ?? 64
  const stuck = ref(true)
  /// Set when `scrollToBottom` writes `scrollTop` and the assignment
  /// will actually move the scroll position (i.e. fire a `scroll`
  /// event). Cleared by the next `onScroll` handler. Without this
  /// guard, the scroll event from programmatic scroll-to-bottom races
  /// against post-paint content growth (a ToolPill auto-expanding
  /// when its state flips to `running` adds dozens of pixels AFTER
  /// the assignment, but BEFORE the scroll event fires) — `nearBottom`
  /// then sees the now-stale scrollTop vs the new scrollHeight and
  /// flips `stuck=false`, breaking the auto-follow.
  let suppressNextScrollUpdate = false

  function nearBottom(el: HTMLElement): boolean {
    return el.scrollHeight - el.scrollTop - el.clientHeight <= threshold
  }

  function scrollToBottom(): void {
    const el = scrollEl.value

    if (!el) {
      return
    }
    // Only set the suppress flag when the assignment will actually
    // change scrollTop — otherwise no scroll event fires and the flag
    // would stick across the next user-initiated scroll.
    const target = el.scrollHeight - el.clientHeight

    if (el.scrollTop !== target) {
      suppressNextScrollUpdate = true
    }

    // jsdom test paths sometimes redefine `scrollTop` as a non-writable
    // property to simulate scroll positions; an rAF callback queued
    // before that redefinition then throws when it fires post-test.
    // Real browsers never lock down `scrollTop` so the try/catch is a
    // no-op in production. Without it, vitest catches the post-test
    // throw and CI exits 1 even though every assertion passed.
    try {
      el.scrollTop = el.scrollHeight
    } catch {
      // Assignment failed — the scroll event won't fire, so don't
      // leave the suppress flag set.
      suppressNextScrollUpdate = false
    }
  }

  function onScroll(): void {
    const el = scrollEl.value

    if (!el) {
      return
    }

    if (suppressNextScrollUpdate) {
      // Programmatic scroll-to-bottom — the scroll event we're seeing
      // here was caused by us, not the captain. Two things matter:
      //
      // 1. Don't let post-scroll content growth (a ToolPill auto-
      //    expanding when state flips to `running` adds px AFTER the
      //    assignment) flip stuck=false. That's the original race.
      //
      // 2. DO re-establish stuck=true. The chevron click case proves
      //    why: when the captain is scrolled away (stuck=false) and
      //    clicks the down arrow, `scrollToBottom()` lands them at
      //    the foot, the suppress branch swallows the scroll event
      //    — but if we just bailed without writing stuck, it would
      //    stay false and the next streaming chunk's MutationObserver
      //    would early-return in scheduleStick. Auto-follow stays
      //    dead until the captain manually scrolls within 64px.
      //
      // Forcing stuck=true here matches captain intent: a programmatic
      // scroll-to-bottom is always an explicit "follow the live tail".
      suppressNextScrollUpdate = false
      stuck.value = true

      return
    }
    stuck.value = nearBottom(el)
  }

  let resizeObs: ResizeObserver | undefined
  let mutationObs: MutationObserver | undefined

  // rAF coalescing — observers fire per text mutation during
  // streaming (one per chunk × N children). Each callback synchronously
  // reads `scrollHeight` / `scrollTop` / `clientHeight` then writes
  // `scrollTop`, forcing a layout flush per chunk. Coalescing to one
  // rAF tick per burst collapses 50 layout flushes/sec to ~60Hz max.
  let rafPending = false
  let rafHandle: number | undefined

  function scheduleStick(): void {
    if (rafPending) {
      return
    }

    if (!stuck.value) {
      return
    }
    rafPending = true
    rafHandle = requestAnimationFrame(() => {
      rafPending = false
      rafHandle = undefined

      if (stuck.value) {
        scrollToBottom()
      }
    })
  }

  onMounted(() => {
    const el = scrollEl.value

    if (!el) {
      return
    }
    el.addEventListener('scroll', onScroll, { passive: true })

    // jsdom (vitest) doesn't ship `ResizeObserver` / `MutationObserver`
    // — guard so component tests mounting the parent don't crash. The
    // observers are runtime-only enhancements; without them the
    // initial scroll-to-bottom still runs and the user can scroll
    // manually.
    if (typeof ResizeObserver !== 'undefined') {
      resizeObs = new ResizeObserver(scheduleStick)
      resizeObs.observe(el)
    }

    if (typeof MutationObserver !== 'undefined') {
      mutationObs = new MutationObserver(scheduleStick)
      mutationObs.observe(el, {
        childList: true,
        subtree: true,
        characterData: true
      })
    }

    scrollToBottom()
  })

  onUnmounted(() => {
    const el = scrollEl.value

    if (el) {
      el.removeEventListener('scroll', onScroll)
    }
    resizeObs?.disconnect()
    mutationObs?.disconnect()

    if (rafHandle !== undefined) {
      cancelAnimationFrame(rafHandle)
      rafHandle = undefined
      rafPending = false
    }
  })

  return { stuck, scrollToBottom }
}
