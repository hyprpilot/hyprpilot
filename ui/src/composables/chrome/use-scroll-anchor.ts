/**
 * Scroll-anchor primitive for the chat viewport.
 *
 * Browser scrollTop is pixel-based; chat content's natural reference
 * is row-based. When content above the captain's view resizes
 * (streaming chunks grow, Shiki finishes highlighting, image loads,
 * pages prepend), scrollTop stays constant but the content under the
 * captain's eye shifts — the scrollbar lies, and reading position
 * drifts. The fix: track the captain's anchor as `{ rowSeq,
 * offsetWithinRow }` and re-lock scrollTop whenever any layout-
 * shifting event fires above it.
 *
 * Row identity is the daemon-minted seq (carried on each `<Turn>` via
 * `data-anchor-seq`). Seq is stable across re-renders, page evictions,
 * folding, and instance flips — pixel offset is what's broken today;
 * Vue keys (groupKey) can rebuild when entries fold/unfold.
 *
 * **Mechanism**:
 *
 *   1. `scroll` handler (rAF-throttled, passive) walks rows and picks
 *      the topmost whose `offsetTop + offsetHeight > scrollTop` as
 *      the anchor. Stores `{ rowSeq, offsetWithinRow }`.
 *   2. `ResizeObserver` on the scroll container (content-box) fires
 *      after every layout-shifting event. On fire, look up the anchor
 *      row's NEW offsetTop, set `scrollTop = newTop + offsetWithinRow`.
 *   3. `programmaticScroll` flag is set right before our own writes
 *      and cleared after 500ms or `scrollend` (when available). The
 *      flag suppresses anchor capture during our motion so we don't
 *      feedback-loop.
 *
 * **Coexistence with `useStickToBottom`**: when `stuck=true`, anchor
 * is dormant — foot-follow owns scroll. When `stuck=false`, anchor
 * is active. The transition `true → false` (captain scrolls up)
 * captures the anchor immediately so the very next resize re-locks
 * to where the captain is.
 *
 * **WebKitGTK async wheel race**: WebKit2GTK delivers wheel-driven
 * scroll events asynchronously via the compositor. If the captain
 * wheels up while a streaming chunk lands, the rAF-throttled scroll
 * handler runs *after* the resize event fires — the anchor would be
 * captured at the pre-wheel position. `releaseAnchor()` is the
 * synchronous escape hatch: gesture handlers in `Viewport.vue` call
 * it on `wheel.deltaY < 0` BEFORE the gesture's first scroll event
 * lands, marking the anchor stale so the next resize doesn't yank.
 */

import { onMounted, onUnmounted, ref, watch, type Ref } from 'vue'

/**
 * Captured reading position. `rowSeq` identifies the row via its
 * `data-anchor-seq` attribute; `offsetWithinRow` is `scrollTop -
 * row.offsetTop` (positive — captain's viewport top is below the
 * row's top).
 */
export interface ScrollAnchor {
  rowSeq: number
  offsetWithinRow: number
}

export interface UseScrollAnchorOptions {
  /**
   * Source-of-truth signal from `useStickToBottom`. When `true`,
   * anchor goes idle — the foot-follow path owns scroll. When
   * `false`, anchor captures + re-locks on every resize.
   */
  stuck: Ref<boolean>
  /**
   * CSS selector for anchor-bearing rows within the scroll container.
   * Each match must carry a `data-anchor-seq` attribute. Defaults to
   * `[data-anchor-seq]`.
   */
  rowSelector?: string
  /**
   * Clear-flag duration for the `programmaticScroll` suppress. 500ms
   * covers smooth-scroll worst case + iOS momentum margin. Falls
   * back to `scrollend` event when supported (Chrome 114+, Firefox
   * 109+, Safari 16.4+) — not in WebKitGTK 4.1 (Tauri's webview), so
   * the timeout is the primary path on desktop overlay.
   */
  programmaticScrollClearMs?: number
}

export interface UseScrollAnchorApi {
  /**
   * Currently captured anchor. `undefined` when `stuck=true` or no
   * rows are visible.
   */
  anchor: Ref<ScrollAnchor | undefined>
  /**
   * Synchronously drop the captured anchor + cancel any pending
   * re-lock. Call from input-gesture handlers BEFORE the gesture
   * starts a programmatic scroll, so a coincident resize-driven
   * re-lock can't pull the captain back. Mirrors
   * `useStickToBottom.release()` — same race-closing pattern.
   */
  releaseAnchor: () => void
  /**
   * Mark the upcoming scroll as our own write (suppresses anchor
   * capture for the next `programmaticScrollClearMs` ms or until
   * `scrollend`). Call before any `scrollTop = N` from a parent
   * composable that wants the anchor to ignore it.
   */
  markProgrammaticScroll: () => void
  /**
   * Find a row by seq and write `scrollTop` so the row's top sits
   * at the requested offset within viewport. Returns `true` when
   * the target row was found, `false` otherwise (row hasn't been
   * fetched yet). Caller's responsibility to handle the false path
   * (typically: trigger a backward fetch + retry).
   */
  scrollToSeq: (seq: number, offsetWithinRow?: number) => boolean
}

const DEFAULT_CLEAR_MS = 500

export function useScrollAnchor(scrollEl: Ref<HTMLElement | undefined>, opts: UseScrollAnchorOptions): UseScrollAnchorApi {
  const anchor = ref<ScrollAnchor | undefined>(undefined)
  const rowSelector = opts.rowSelector ?? '[data-anchor-seq]'
  const clearMs = opts.programmaticScrollClearMs ?? DEFAULT_CLEAR_MS

  let programmaticScroll = false
  let clearTimer: ReturnType<typeof setTimeout> | undefined
  let rafPending = false
  let rafHandle: number | undefined
  let resizeObs: ResizeObserver | undefined
  // Cleanup hook for the scrollend listener — only set when the
  // browser supports `scrollend`. Cleared in `clearProgrammaticFlag`
  // so a second markProgrammaticScroll re-registers cleanly.
  let scrollendCleanup: (() => void) | undefined

  function clearProgrammaticFlag(): void {
    programmaticScroll = false

    if (clearTimer !== undefined) {
      clearTimeout(clearTimer)
      clearTimer = undefined
    }
    scrollendCleanup?.()
    scrollendCleanup = undefined
  }

  function markProgrammaticScroll(): void {
    programmaticScroll = true

    if (clearTimer !== undefined) {
      clearTimeout(clearTimer)
    }
    clearTimer = setTimeout(clearProgrammaticFlag, clearMs)

    // Feature-detect `scrollend` — when present, it clears the flag
    // as soon as the motion actually stops (typically faster than
    // the 500ms timeout fallback). WebKitGTK 4.1 doesn't ship it,
    // so the timeout is the primary path on the Tauri overlay.
    const el = scrollEl.value

    if (el && 'onscrollend' in el) {
      const handler = (): void => {
        clearProgrammaticFlag()
      }

      el.addEventListener('scrollend', handler, { once: true })
      scrollendCleanup = () => el.removeEventListener('scrollend', handler)
    }
  }

  /**
   * Pick the anchor row at the current scroll position. Walks
   * children once — bounded by the daemon transcript ring.
   * Does NOT trigger forced layout: called inside scroll handlers
   * (rAF-throttled) and resize observers, where layout is already
   * clean. Returns `undefined` when no row qualifies (empty viewport,
   * or scrollTop past the last row's bottom).
   */
  function pickAnchorAt(el: HTMLElement, scrollTop: number): ScrollAnchor | undefined {
    const rows = el.querySelectorAll<HTMLElement>(rowSelector)

    for (const row of rows) {
      const rowTop = row.offsetTop
      const rowBottom = rowTop + row.offsetHeight

      if (rowBottom > scrollTop) {
        const seqAttr = row.getAttribute('data-anchor-seq')

        if (seqAttr === null) {
          continue
        }
        const rowSeq = Number(seqAttr)

        if (!Number.isFinite(rowSeq)) {
          continue
        }

        return {
          rowSeq,
          offsetWithinRow: scrollTop - rowTop
        }
      }
    }

    return undefined
  }

  /**
   * Re-lock the captain's scroll position to the anchored row's
   * current pixel position. Called from the ResizeObserver callback
   * after any layout-changing event (streaming chunk growth, Shiki
   * render, image load, page prepend). When the anchor row isn't in
   * the DOM (evicted) we drop the anchor — next scroll captures a
   * fresh one. When stuck=true we bail; foot-follow owns scroll.
   */
  function relock(): void {
    const el = scrollEl.value
    const a = anchor.value

    if (!el || !a || opts.stuck.value || programmaticScroll) {
      return
    }
    const row = el.querySelector<HTMLElement>(`[data-anchor-seq="${a.rowSeq}"]`)

    if (!row) {
      // Anchor row was evicted from the DOM. Drop the anchor — the
      // next scroll event will pick a fresh one from whatever rows
      // are present. Captain's scrollTop stays where the browser
      // left it; without a known target we can't compute a better
      // position. This branch is rare (eviction is gated on
      // stuck=true) but defensive.
      anchor.value = undefined

      return
    }
    const target = row.offsetTop + a.offsetWithinRow

    // Tolerance avoids fighting sub-pixel drift on high-DPI displays.
    if (Math.abs(el.scrollTop - target) > 0.5) {
      markProgrammaticScroll()
      el.scrollTop = target
    }
  }

  function onScroll(): void {
    const el = scrollEl.value

    if (!el) {
      return
    }

    if (programmaticScroll) {
      return
    }

    if (opts.stuck.value) {
      // Foot-follow owns scroll. Keep anchor undefined so a
      // subsequent stuck → unstuck transition captures fresh.
      anchor.value = undefined

      return
    }

    if (rafPending) {
      return
    }
    rafPending = true
    rafHandle = requestAnimationFrame(() => {
      rafPending = false
      rafHandle = undefined

      // Re-check inside the rAF — a release / stuck-flip may have
      // happened in the window between schedule and fire.
      if (programmaticScroll || opts.stuck.value || !scrollEl.value) {
        return
      }
      const picked = pickAnchorAt(scrollEl.value, scrollEl.value.scrollTop)

      if (picked) {
        anchor.value = picked
      }
    })
  }

  function releaseAnchor(): void {
    anchor.value = undefined

    if (rafHandle !== undefined) {
      cancelAnimationFrame(rafHandle)
      rafHandle = undefined
      rafPending = false
    }
    // Suppress capture across the WebKitGTK compositor's async-wheel
    // delivery window. Without this, the compositor's delayed scroll
    // event lands AFTER `releaseAnchor` fires; `onScroll` runs and
    // captures a new anchor at the still-pre-gesture position. The
    // very next resize re-locks to that stale anchor and pulls the
    // captain back — the exact race this primitive's `releaseAnchor`
    // is meant to close. Marking programmatic-scroll covers the gap
    // by suppressing capture for `programmaticScrollClearMs` (500ms
    // default) — long enough for any compositor delivery to land
    // and be ignored.
    markProgrammaticScroll()
  }

  function scrollToSeq(seq: number, offsetWithinRow = 0): boolean {
    const el = scrollEl.value

    if (!el) {
      return false
    }
    const row = el.querySelector<HTMLElement>(`[data-anchor-seq="${seq}"]`)

    if (!row) {
      return false
    }
    markProgrammaticScroll()
    el.scrollTop = row.offsetTop + offsetWithinRow

    return true
  }

  // Top-level watcher — `stuck` is a Vue ref and doesn't need the
  // scroll element to exist for the watch to register. Keeping it
  // outside `onMounted` avoids a silent registration failure if
  // `scrollEl.value` is undefined at mount time (lazy ref
  // assignment). The watch body itself reads `scrollEl.value`
  // defensively before pickAnchorAt.
  //
  // When stuck flips true (captain returned to foot), drop the
  // anchor — foot-follow path owns scroll. When it flips false
  // (captain scrolled up), capture immediately so the first
  // resize re-locks to the right row.
  watch(opts.stuck, (next) => {
    if (next) {
      anchor.value = undefined
    } else if (scrollEl.value) {
      const picked = pickAnchorAt(scrollEl.value, scrollEl.value.scrollTop)

      if (picked) {
        anchor.value = picked
      }
    }
  })

  onMounted(() => {
    const el = scrollEl.value

    if (!el) {
      return
    }
    el.addEventListener('scroll', onScroll, { passive: true })

    // jsdom doesn't ship ResizeObserver — runtime-only enhancement,
    // tests run with anchor.value=undefined which is fine.
    if (typeof ResizeObserver !== 'undefined') {
      resizeObs = new ResizeObserver(relock)
      // `box: 'content-box'` is the default for `observe()`, but
      // calling it out explicitly so a future reader doesn't wonder
      // whether scrollbar-width changes are part of the trigger set.
      // They are NOT — content-box ignores them.
      resizeObs.observe(el, { box: 'content-box' })
    }
  })

  onUnmounted(() => {
    const el = scrollEl.value

    if (el) {
      el.removeEventListener('scroll', onScroll)
    }
    resizeObs?.disconnect()

    if (rafHandle !== undefined) {
      cancelAnimationFrame(rafHandle)
    }
    clearProgrammaticFlag()
  })

  return {
    anchor,
    releaseAnchor,
    markProgrammaticScroll,
    scrollToSeq
  }
}
