import { onMounted, onUnmounted, ref, type Ref } from 'vue'

const SETTLE_FRAME_COUNT = 4

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
 *   - `ResizeObserver` catches the content wrapper growing/shrinking
 *     (long-running tool output growing inline, code blocks
 *     expanding, etc.) which `MutationObserver` doesn't.
 *
 * `stuck` is exposed for callers that want a "scroll to bottom"
 * affordance — `false` means the user has scrolled away.
 */
export function useStickToBottom(
  scrollEl: Ref<HTMLElement | undefined>,
  options?: { threshold?: number; observeEl?: Ref<HTMLElement | undefined> }
): {
  stuck: Ref<boolean>
  scrollToBottom: () => void
  release: () => void
} {
  // Captain-tuned default. The earlier 64px was tight enough that a
  // single thumb-swipe on mobile or a small trackpad nudge would
  // reliably overshoot and unstick the viewport — captain reported
  // stick-to-bottom as "a bit unreliable now". 128px is roughly half
  // a screenful of breathing room without making the chevron hide
  // too eagerly (192 was the first attempt; captain tuned down to
  // 128). The chevron at `Viewport.vue::scroll-to-bottom` reads the
  // same `stuck` signal — "not snapped in ⇒ show the button" — so
  // both behaviors stay aligned by virtue of the shared signal.
  const threshold = options?.threshold ?? 128
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
  /// Previous `scrollTop` observed by `onScroll`. Drives upward-
  /// gesture detection — any non-suppressed scroll event whose
  /// `scrollTop` is lower than `prevScrollTop` flips `stuck = false`
  /// immediately, without waiting for the stick threshold. Captures
  /// the OS scrollbar drag (native widget — fires `scroll` but NOT
  /// `pointerdown` on the DOM element) and prevents small-wheel-up
  /// snap-back during streaming (a 20px wheel-up wouldn't cross the
  /// threshold; the next MutationObserver-driven `scheduleStick`
  /// would still see `stuck=true` and yank the captain back to the
  /// foot). Initialised to 0; the first `scroll` event after mount
  /// resets it through the suppress branch.
  let prevScrollTop = 0
  let prevScrollHeight = 0
  let prevClientHeight = 0

  // rAF coalescing — observers fire per text mutation during
  // streaming (one per chunk × N children). Each callback synchronously
  // reads `scrollHeight` / `scrollTop` / `clientHeight` then writes
  // `scrollTop`, forcing a layout flush per chunk. Coalescing to one
  // rAF tick per burst collapses 50 layout flushes/sec to ~60Hz max.
  // Declared above `onScroll` so that handler can cancel a pending
  // rAF when an upward gesture lands inside the schedule→fire window.
  let rafPending = false
  let rafHandle: number | undefined
  let settleFramesRemaining = 0

  function cancelStickFrame(): void {
    if (rafHandle === undefined) {
      return
    }
    cancelAnimationFrame(rafHandle)
    rafHandle = undefined
    rafPending = false
    settleFramesRemaining = 0
  }

  function nearBottom(el: HTMLElement): boolean {
    return el.scrollHeight - el.scrollTop - el.clientHeight <= threshold
  }

  function maxScrollTop(scrollHeight: number, clientHeight: number): number {
    return Math.max(0, scrollHeight - clientHeight)
  }

  function atScrollLimit(scrollTop: number, max: number): boolean {
    return scrollTop >= max - 1
  }

  function scrollToBottom(): void {
    const el = scrollEl.value

    if (!el) {
      return
    }
    // Only set the suppress flag when the assignment will actually
    // change scrollTop — otherwise no scroll event fires and the flag
    // would stick across the next user-initiated scroll.
    const target = maxScrollTop(el.scrollHeight, el.clientHeight)

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
      el.scrollTop = target
      prevScrollTop = el.scrollTop
      prevScrollHeight = el.scrollHeight
      prevClientHeight = el.clientHeight
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
    const current = el.scrollTop
    const oldScrollHeight = prevScrollHeight
    const oldClientHeight = prevClientHeight
    const layoutChanged = el.scrollHeight !== oldScrollHeight || el.clientHeight !== oldClientHeight
    const newMaxScrollTop = maxScrollTop(el.scrollHeight, el.clientHeight)

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
      prevScrollTop = current
      prevScrollHeight = el.scrollHeight
      prevClientHeight = el.clientHeight

      return
    }
    // Upward gesture? Exit stick mode immediately — don't wait for the
    // stick threshold. Catches both:
    //   1. Wheel-up of any size, even a single small notch (the captain
    //      reported the chat snapping back to the foot when they nudged
    //      the wheel up during streaming because the gesture didn't
    //      cross the threshold).
    //   2. OS scrollbar drag toward the top. The native scrollbar is a
    //      widget pseudo-element — dragging it fires `scroll` events
    //      on this element but no preceding `pointerdown` /
    //      `touchstart` / `wheel`, so the only signal we get that the
    //      captain is reading older content is the decreasing
    //      `scrollTop`.
    // Downward / unchanged: fall through to the threshold check so a
    // following live tail with a brief overshoot doesn't bounce
    // `stuck` off.
    const movedUp = current < prevScrollTop

    if (oldScrollHeight === 0 && oldClientHeight === 0) {
      stuck.value = nearBottom(el)
      prevScrollTop = current
      prevScrollHeight = el.scrollHeight
      prevClientHeight = el.clientHeight

      return
    }

    prevScrollTop = current
    prevScrollHeight = el.scrollHeight
    prevClientHeight = el.clientHeight

    if (stuck.value && layoutChanged && movedUp && atScrollLimit(current, newMaxScrollTop)) {
      // Layout-driven clamp while we were already auto-following, not
      // captain intent. This happens when a long composer prompt
      // clears, the viewport grows, and the browser lowers scrollTop to
      // the new maximum. Keep the latch engaged and leave any pending
      // stick rAF alive so it can re-close the bottom gap after the
      // DOM settles.
      scheduleStick()

      return
    }

    if (movedUp) {
      stuck.value = false

      // Cancel any in-flight `scheduleStick` rAF. The chip's elapsed
      // counter (and any live tail surface) mutates text every second
      // — each tick queues a rAF. If the captain wheels up / hits
      // PageUp in the ~16ms window before the queued rAF fires, the
      // rAF still runs `scrollToBottom` (the comment on `scheduleStick`
      // explains why we don't re-check `stuck.value` at fire time —
      // that re-check caused the spurious-reflow-on-submit bug). The
      // cancel here closes the race in the OTHER direction: when the
      // movement is explicitly upward, the captain's intent
      // overrides whatever the schedule-time decision was.
      cancelStickFrame()

      return
    }

    if (stuck.value && layoutChanged) {
      // Still in auto-follow mode and the scroll did not move upward.
      // A large append can fire a browser scroll event before the
      // MutationObserver / ResizeObserver pass gets to pull us back to
      // the foot. In that transient frame `nearBottom` may be false by
      // more than the threshold, but there was no upward captain
      // gesture. Keep the latch engaged so the already-scheduled (or
      // soon-to-be scheduled) stick pass continues following the live
      // tail.
      scheduleStick()

      return
    }
    stuck.value = nearBottom(el)
  }

  let resizeObs: ResizeObserver | undefined
  let mutationObs: MutationObserver | undefined

  function scheduleStick(settleFrames = SETTLE_FRAME_COUNT): void {
    settleFramesRemaining = Math.max(settleFramesRemaining, settleFrames)

    if (rafPending) {
      return
    }

    if (!stuck.value) {
      settleFramesRemaining = 0

      return
    }
    rafPending = true
    rafHandle = requestAnimationFrame(() => {
      rafPending = false
      rafHandle = undefined

      // Engagement was decided at schedule time (line above —
      // `if (!stuck.value) return`). Trust that decision; do NOT
      // re-check `stuck.value` here. Captain-reported bug: sending a
      // message while already at the bottom sometimes lost
      // stickiness. The cause was a race in the ~16 ms gap between
      // schedule and rAF:
      //
      //   1. New DOM child appended at the foot → MutationObserver
      //      fires → schedule (stuck still true).
      //   2. Browser-internal reflow side-effects (composer below
      //      the viewport shrinking after submit clears the textarea
      //      → viewport `clientHeight` grows → `scrollTop` clamped
      //      down by the browser → `scroll` event fires).
      //   3. `onScroll` sees `nearBottom = false` for a single frame
      //      because the chunk that landed in step 1 hasn't fully
      //      laid out yet → flips `stuck = false`.
      //   4. rAF fires, re-checks `stuck.value` → false → skips
      //      `scrollToBottom`.
      //   5. Captain is now stranded at the previous position with
      //      auto-follow dead.
      //
      // The fix: once engaged, always scroll. `scrollToBottom`'s
      // `suppressNextScrollUpdate` flag (see top of file) absorbs
      // the resulting scroll event and forces `stuck = true` back
      // on. Any concurrent captain wheel/touch within 16 ms is
      // vanishingly rare in practice; re-establishing stuck on a
      // mutation-driven engage matches the captain's intent ("I was
      // at the bottom; keep me there as content streams in").
      scrollToBottom()

      // DOM/layout can keep growing for a few frames after the first
      // mutation: long prompts resize the composer, streamed thoughts
      // expand nested cards, and Shiki/code blocks swap in highlighted
      // DOM after the markdown node already landed. Observers do not
      // reliably fire for every scrollHeight-only change, so keep a
      // short settle loop while the latch is still engaged. User upward
      // gestures cancel the loop via `release()` / the movedUp branch.
      if (stuck.value && settleFramesRemaining > 0) {
        settleFramesRemaining -= 1
        scheduleStick(0)
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
    const observedEl = options?.observeEl?.value ?? el

    if (typeof ResizeObserver !== 'undefined') {
      resizeObs = new ResizeObserver(() => scheduleStick())
      resizeObs.observe(observedEl)
    }

    if (typeof MutationObserver !== 'undefined') {
      mutationObs = new MutationObserver(() => scheduleStick())
      mutationObs.observe(observedEl, {
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

    cancelStickFrame()
  })

  /// Synchronously release auto-follow + cancel any in-flight
  /// stick rAF. Call from input handlers BEFORE initiating an
  /// upward scroll so a coincident mutation-driven rAF can't
  /// snap the viewport back to the foot.
  ///
  /// Why this exists separately from `onScroll`'s direction-based
  /// cancel: input gestures that use `behavior: 'smooth'` (PageUp,
  /// Home, the `scrollBy` paths in `Viewport.vue`) don't fire their
  /// first scroll event until the next animation frame — by then a
  /// rAF queued from a coincident chunk has already fired and run
  /// `scrollToBottom`, which cancels the smooth scroll. The
  /// captain's gesture vanishes silently. Same race with wheel on
  /// compositor-accelerated scrolling (WebKit2GTK delivers wheel-
  /// driven scroll events asynchronously via the compositor). The
  /// only bulletproof fix is for the gesture handler to release
  /// stick SYNCHRONOUSLY at gesture-start, before the rAF gets a
  /// chance to fire.
  function release(): void {
    stuck.value = false
    // Clear the suppress flag too. Narrow but real race: if
    // `scrollToBottom` ran moments before the captain's input AND
    // its scroll event hasn't been delivered yet, the next
    // `onScroll` would consume the suppress flag and force
    // `stuck = true` — undoing this release for one tick. The
    // captain's smooth scroll's own scroll events would then
    // re-unstick via `movedUp`, but the brief re-engage looks like
    // the hostage bug. Clearing the flag here means the very next
    // scroll event runs through the normal `movedUp` branch.
    suppressNextScrollUpdate = false

    cancelStickFrame()
  }

  return {
    stuck,
    scrollToBottom,
    release
  }
}
