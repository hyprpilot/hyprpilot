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

  function nearBottom(el: HTMLElement): boolean {
    return el.scrollHeight - el.scrollTop - el.clientHeight <= threshold
  }

  function scrollToBottom(): void {
    const el = scrollEl.value

    if (!el) {
      return
    }
    el.scrollTop = el.scrollHeight
  }

  function onScroll(): void {
    const el = scrollEl.value

    if (!el) {
      return
    }
    stuck.value = nearBottom(el)
  }

  let resizeObs: ResizeObserver | undefined
  let mutationObs: MutationObserver | undefined

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
      resizeObs = new ResizeObserver(() => {
        if (stuck.value) {
          scrollToBottom()
        }
      })
      resizeObs.observe(el)
    }

    if (typeof MutationObserver !== 'undefined') {
      mutationObs = new MutationObserver(() => {
        if (stuck.value) {
          scrollToBottom()
        }
      })
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
  })

  return { stuck, scrollToBottom }
}
