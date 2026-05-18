<script setup lang="ts">
/**
 * Chat transcript viewport. Reads off `useChatViewport` (which wraps
 * `useInstanceChatInfiniteQuery` + live-event patches + page-trim
 * policy) and feeds the resulting blocks into a plain `v-for` render.
 *
 * **No virtualization.** TanStack Vue Virtual was tried twice and
 * pulled out twice. Variable-height content + streaming chunks
 * creates a tight ResizeObserver / `triggerRef` / re-measure cycle
 * that never converges:
 *
 *   1. virtualizer.measureElement(row) → onChange → triggerRef(state)
 *   2. virtualRows recomputes (positions shift)
 *   3. Vue re-renders the row, content unchanged but ref re-fires
 *   4. ResizeObserver fires for the changed-size row (head row keeps
 *      growing during streaming)
 *   5. Goto 1
 *
 * Vue caps the loop at 100 iterations and throws "Maximum recursive
 * updates exceeded in component <Viewport>". The non-virtualized
 * `v-for` is stable: page-trim already bounds the live DOM to ~150
 * rows (`useChatViewport.MAX_PAGES_KEPT` × `DEFAULT_CHAT_LIMIT`),
 * which Vue handles without breaking a sweat. Re-virtualize later
 * if a real memory ceiling shows up; the bottleneck today is
 * correctness.
 *
 * **Backward pagination**: a `@scroll` handler watches `scrollTop` and
 * triggers `viewport.fetchNextPage()` when the captain crosses
 * `LOAD_MORE_THRESHOLD_PX` from the top.
 *
 * **Stick-to-bottom**: `useStickToBottom` already runs a
 * MutationObserver + ResizeObserver pair on the scroll container and
 * scrolls to the tail on every mutation while `stuck` is true. No
 * extra Vue watcher needed.
 */
import { faChevronDown } from '@fortawesome/free-solid-svg-icons'
import { useEventListener, useIntersectionObserver, useNow } from '@vueuse/core'
import { computed, nextTick, ref, watch } from 'vue'

import Attachments from './Attachments.vue'
import Body from './Body.vue'
import ChangeBanner from './ChangeBanner.vue'
import StreamCard from './StreamCard.vue'
import TerminalCard from './TerminalCard.vue'
import ToolChips from './ToolChips.vue'
import Turn from './Turn.vue'
import { Loading, Role, StreamKind, PlanStatus, type PlanItem } from '@components'
import {
  isEditableTarget,
  StreamItemKind,
  TurnRole,
  timelineBlocksFromSnapshot,
  useActiveInstance,
  useAgentRegistry,
  useChatViewport,
  useSessionInfo,
  useSnapshotHydration,
  useStickToBottom,
  useTurns,
  type PlanEntry,
  type WireToolCall,
  type InstanceId
} from '@composables'
import { format, formatDuration, log } from '@lib'

/// Sentinel `rootMargin` for the intersection-observer-driven
/// backward fetch. The sentinel renders 0-height at the top of the
/// list; expanding its observed root margin upward by this much
/// pre-fetches before the captain visually reaches the top, so the
/// chip and the new content land while the captain is still
/// scrolling. Replaces the prior `LOAD_MORE_THRESHOLD_PX` scroll-
/// event check — sentinels fire once per visibility transition (no
/// inertia double-fire), don't compete with scroll-event throttling,
/// and don't need a manual `loadingEarlier` dedup flag because
/// vue-query's `isFetchingNextPage` is the truth.
const LOAD_MORE_SENTINEL_MARGIN_PX = 240

const props = defineProps<{
  /// Captain's "session is restoring" gate — keeps the scoped
  /// `<Loading>` overlay painted on top while transcript replay is
  /// in flight.
  restoring?: boolean
}>()

const emit = defineEmits<{
  /// Fires when the captain hits the per-tool-call cancel button on
  /// a live terminal — Overlay.vue routes this to `session_cancel`.
  cancel: []
  /// Fires when an attachment pill on a captain turn is clicked.
  'attachment-open': [attachment: import('@ipc').Attachment]
}>()

const { id: activeInstanceId } = useActiveInstance()
const instanceId = computed<InstanceId | undefined>(() => activeInstanceId.value)

// `scrollEl` declared up-front so `useChatViewport` can derive its
// fetch page size from `clientHeight`. The ref is undefined until
// mount; `viewportPageSize`'s fallback returns the minimum size so
// the initial fetch races a sane lower bound, and every backward
// page picks up the real viewport extent.
const scrollEl = ref<HTMLElement>()
/// Flipped on the first captain-initiated input gesture after mount.
/// Both `fetchNextPage` (load older) and `evictExtraPages` (drop
/// trailing pages) gate on this so they don't fire during the
/// initial paint of a freshly-mounted viewport.
///
/// **Why a gesture listener, not a scroll-event heuristic.**
/// `useStickToBottom.onMounted` writes `scrollTop = scrollHeight` on
/// mount, and its MutationObserver re-stick passes write again on
/// every chunk while the head page renders. The browser fires
/// synthetic `scroll` events for each — indistinguishable from a
/// captain's drag if you only watch `scrollTop`. A distance-based
/// gate (the previous shape) had two failure modes: (1) on a small
/// upward drag, `distanceFromBottom < threshold` so the gate stayed
/// locked, burning the gesture; (2) on a large drag past the
/// threshold, the gate unlocked but the same scroll tick also
/// satisfied the load-more condition, racing the fetch.
///
/// `wheel` / `touchstart` / `pointerdown` are pure intent signals —
/// stick-to-bottom never fires them. First one flips the gate; from
/// that point on, every scroll handler tick can pull older pages.
///
/// Resets to `false` on every mount (the `:key="activeInstanceId"`
/// on `<ChatViewport>` in `Overlay.vue` forces a clean remount per
/// instance flip, so this ref is freshly `false` for each instance).
const hasUserScrolled = ref(false)

function markUserScrolled(): void {
  if (!hasUserScrolled.value) {
    hasUserScrolled.value = true
  }
}

// `wheel` with negative deltaY (or trackpad scroll-up) releases
// stick SYNCHRONOUSLY before WebKit2GTK's compositor delivers the
// async scroll event a few frames later. End-of-frame races where a
// chunk-driven rAF fired first cancel the captain's scroll silently
// without this. Downward wheel falls through to the same
// markUserScrolled intent without release.
useEventListener(
  scrollEl,
  'wheel',
  (ev: WheelEvent) => {
    if (ev.deltaY < 0) {
      releaseStickAndMark()
    } else {
      markUserScrolled()
    }
  },
  { passive: true }
)
useEventListener(scrollEl, 'touchstart', markUserScrolled, { passive: true })
useEventListener(scrollEl, 'pointerdown', markUserScrolled, { passive: true })

// Mobile / touch path: release stick on the FIRST touchmove. `touchmove`
// only fires when the captain's finger actually moves (a tap that
// completes without movement never fires it) — so this is the touch
// equivalent of the upward-wheel release on desktop. Required because
// of a MutationObserver-vs-touch race specific to mobile webviews:
//
//   1. Live chunk lands → `MutationObserver` → `scheduleStick` → rAF
//      queued (stuck is still true).
//   2. Captain's finger starts moving (upward swipe to read older).
//   3. rAF fires BEFORE the captain's first `scroll` event lands —
//      iOS Safari / Chrome Android throttle inertia scroll events
//      relative to the gesture, but the rAF clock is unaffected.
//      `scrollToBottom()` writes `scrollTop = scrollHeight` and the
//      captain's swipe is silently cancelled (the synthetic scroll
//      consumes `suppressNextScrollUpdate` + forces stuck=true again).
//   4. Captain's next scroll events fire AFTER the snap-back, with
//      `prevScrollTop` re-baselined at the foot — `movedUp` never
//      flips, stick stays engaged. Captain reports "I can't break
//      the lock on mobile."
//
// Releasing stick synchronously at the first `touchmove` closes the
// race the same way `wheel.deltaY<0` does on desktop: the queued rAF
// short-circuits at `!stuck.value` when it fires, so the snap-back
// never happens. Taps without movement don't fire `touchmove`, so the
// stick stays engaged through tap-to-open-attachment / tap-to-copy /
// long-press-context-menu interactions.
useEventListener(scrollEl, 'touchmove', releaseStickAndMark, { passive: true })

const viewport = useChatViewport(instanceId, { scrollEl })
const blocks = computed(() => timelineBlocksFromSnapshot(viewport.items.value, instanceId.value ?? 'snapshot'))

const { adapterFor } = useAgentRegistry()
const { info: sessionInfo } = useSessionInfo()
const { openTurnId, turns: turnRecords } = useTurns()

// Hydrate `useTurns` from the snapshot meta query. Live events that
// streamed before this component mounted (focus-switch, remote bridge
// authenticated mid-session) are invisible to the live router; the
// daemon mirror has them and ships them on `MetaSnapshot.turns`.
useSnapshotHydration(instanceId)

// Resolve adapter for the active instance's agent. Snapshot tool-call
// entries don't carry agentId on the wire; we look it up off the
// active session's meta so the formatter can produce icons +
// state-aware stats.
const adapterForActive = computed(() => {
  const id = sessionInfo.value.agent

  return id ? adapterFor(id) : undefined
})

const { stuck, scrollToBottom, release: releaseStick } = useStickToBottom(scrollEl)

/// Per-mount "first hydration" latch. `useStickToBottom` runs a
/// `scrollToBottom` in its own `onMounted`, but that fires BEFORE
/// the chat snapshot lands — at that moment `viewport.items.value`
/// is empty and `scrollHeight === clientHeight`, so the assignment
/// is a no-op. The MutationObserver-driven re-stick that follows
/// catches most subsequent mutations, but a fully-cached snapshot
/// (the captain returns to a previously-focused instance and the
/// query cache is still warm) renders in one Vue tick with no
/// observable DOM mutation between empty + populated — the
/// `MutationObserver` callback runs, but `scrollHeight` may already
/// equal `scrollTop + clientHeight` from the prior assignment, so
/// `scheduleStick`'s rAF coalescing decides there's nothing to do.
///
/// Captain reported: switching instances via the palette list,
/// the chat lands part-way up instead of at the foot. This watcher
/// closes the gap explicitly — the first transition from "no items"
/// to "items present" per mount triggers an explicit
/// `scrollToBottom` after Vue has flushed the DOM, regardless of
/// whether the MutationObserver fired. `<ChatViewport>` is keyed
/// on `activeInstanceId` in `Overlay.vue`, so this watcher fires
/// once per instance-flip.
let firstHydrationLanded = false

watch(
  () => viewport.items.value.length,
  async(count) => {
    if (firstHydrationLanded || count === 0) {
      return
    }
    firstHydrationLanded = true
    await nextTick()
    // Two ticks: the first lets Vue flush the items[] update into
    // the DOM; the second covers any nested `<Turn>` / `<StreamCard>`
    // child watchers that run their own `nextTick` for layout
    // measurement (markdown render passes, syntax-highlight swaps).
    await nextTick()
    scrollToBottom()
  },
  { immediate: true }
)

/// Synchronous "captain wants to scroll up" gate. Calls into the
/// composable to cancel any pending sticky rAF + flip `stuck =
/// false` BEFORE the input gesture initiates its scroll. Without
/// this, gestures using `behavior: 'smooth'` (PageUp / Home, the
/// keyboard handlers below) — or wheel scrolls delivered async by
/// WebKit2GTK's compositor — get cancelled by a coincident
/// MutationObserver-driven `scrollToBottom` rAF firing first.
/// Captain reported the viewport "stays hostage" when stuck at
/// the bottom; this is the fix.
function releaseStickAndMark(): void {
  markUserScrolled()
  releaseStick()
}

// `stuck` is the auto-scroll signal — strict 64px-from-bottom
// threshold so a captain reading 1 viewport above the foot
// doesn't get yanked back on every chunk. We do NOT use it for
// eviction; eviction fires from `onScroll` whenever the captain
// is within ~one viewport of the bottom, which is wider than the
// auto-scroll window so cache cleanup is prompt without
// disturbing the read-history flow.

// **Window-focus → scroll-to-end.** Captain explicitly asked: when
// they switch away from the overlay (alt-tab, Hyprland keybind
// hide/show, browser tab swap on remote) and come back, the chat
// surface should be at the latest message, not wherever the cursor
// was. Tauri 2 propagates window focus to the DOM `focus` event;
// the remote-bridge browser context already fires it natively. We
// defer to `nextTick` so the layout has a chance to settle —
// `scrollHeight` could be stale if a `useStickToBottom` MutationObserver
// pass is queued from chunks that landed while we were unfocused.
useEventListener(window, 'focus', () => {
  void nextTick(() => {
    scrollToBottom()
  })
})

/// Floating chevron click — drop extra pages, await Vue's DOM
/// patch, THEN jump to the foot. Eviction shrinks `data.pages`
/// from the OLDEST entry, which renders at the TOP of the DOM
/// (`use-chat-viewport.items` walks pages last→first).
/// `evictExtraPages()` calls `setQueryData` synchronously, but
/// Vue's reactive DOM patch flushes on the next microtask — so
/// without `await nextTick()`, `scrollToBottom()` would read the
/// PRE-eviction `scrollHeight`, scroll past the new tail, and the
/// browser would clamp scrollTop in a visible second step.
/// Awaiting nextTick lets Vue flush the eviction patch first;
/// `scrollToBottom()` then lands exactly at the new bottom in one
/// motion.
async function goToBottom(): Promise<void> {
  viewport.evictExtraPages()
  await nextTick()
  scrollToBottom()
}

// ── Rendering ──────────────────────────────────────────────────────
//
// **No virtualization.** TanStack Vue Virtual was tried twice and
// pulled out twice for the same root cause: variable-height content
// + streaming chunks creates a tight ResizeObserver / `triggerRef` /
// re-measure cycle that never converges:
//
//   1. virtualizer.measureElement(row) → onChange → triggerRef(state)
//   2. virtualRows recomputes (positions shift)
//   3. Vue re-renders the row, content unchanged but ref re-fires
//   4. ResizeObserver fires for the changed-size row (head row keeps
//      growing during streaming)
//   5. Goto 1
//
// Vue caps the loop at 100 iterations and throws "Maximum recursive
// updates exceeded in component <Viewport>". Under streaming reply
// or session/load replay it fires every chunk.
//
// The non-virtualized `v-for` over `blocks` is stable: page-trim
// already bounds the live DOM to ~150 rows
// (`useChatViewport.MAX_PAGES_KEPT` × `DEFAULT_CHAT_LIMIT`), and
// modern Vue can render that without breaking a sweat. The viewport
// can be re-virtualized later if a captain hits a real memory
// ceiling, but the current bottleneck is correctness, not memory.

// ── Keyboard scroll ─────────────────────────────────────────────────
//
// The scroll container is a non-focusable `<div>`, so the browser's
// native PageUp / PageDown / Home / End handling never reaches it —
// those keys only act on the currently-focused element (or the
// document scroll, which the overlay layout doesn't have). We hook
// document-level keydown and translate the navigation keys into
// scroll operations on the transcript, BUT skip when the focus is in
// the composer / palette / any text input so editing keystrokes stay
// untouched. ~90% of the visible scroll viewport is one "page" — that
// matches the desktop convention (slightly less than full-screen so
// context overlaps).
const PAGE_OVERLAP_RATIO = 0.9

useEventListener(document, 'keydown', (ev: KeyboardEvent) => {
  // PageUp / PageDown / Home / End are unambiguously about
  // scrolling a big container — textareas / inputs have no
  // meaningful pageful navigation. The captain's focus lives in
  // the composer textarea 99% of the time, so bailing on
  // `isEditableTarget` for them meant these keys were effectively
  // dead in the captain's primary workflow. Bypass the editable-
  // target gate for ALL viewport nav keys; line navigation inside
  // the composer is rare enough that Ctrl+Home / Ctrl+End cover
  // it (the captain types text, hits Enter, scrolls chat — not
  // mid-document caret hops).
  const key = ev.key
  const isViewportNavKey = key === 'PageUp' || key === 'PageDown' || key === 'Home' || key === 'End'

  if (!isViewportNavKey && isEditableTarget(ev.target)) {
    return
  }
  const el = scrollEl.value

  if (!el) {
    return
  }

  switch (key) {
    case 'PageDown': {
      ev.preventDefault()
      markUserScrolled()
      el.scrollBy({ top: el.clientHeight * PAGE_OVERLAP_RATIO, behavior: 'smooth' })

      return
    }

    case 'PageUp': {
      ev.preventDefault()
      // Upward nav: release stick SYNCHRONOUSLY before scrolling,
      // so a coincident MutationObserver-driven rAF doesn't fire
      // `scrollToBottom` first and cancel the smooth scroll.
      releaseStickAndMark()
      el.scrollBy({ top: -el.clientHeight * PAGE_OVERLAP_RATIO, behavior: 'smooth' })

      return
    }

    case 'Home': {
      ev.preventDefault()
      // Same race as PageUp — release synchronously before the
      // smooth scroll's first frame.
      releaseStickAndMark()
      el.scrollTo({ top: 0, behavior: 'smooth' })

      return
    }

    case 'End': {
      ev.preventDefault()
      markUserScrolled()
      el.scrollTo({ top: el.scrollHeight, behavior: 'smooth' })
      // Smooth-scroll fires `scroll` events along the way, which our
      // `onScroll` handler reacts to — but the timing is browser-
      // dependent and on some phones the final tick lands AFTER the
      // browser settles, leaving a brief window where eviction could
      // be missed. Fire one more pass after a delay covering the
      // typical smooth-scroll duration.
      setTimeout(() => viewport.evictExtraPages(), 350)

      return
    }

    default:
      return
  }
})

/// Top-of-list sentinel — a 0-height marker rendered above the first
/// chat block. `useIntersectionObserver` fires when the sentinel
/// enters the captain's viewport. Drives the backward fetch. Replaces
/// the prior scrollTop-threshold check inside `onScroll`.
const topSentinel = ref<HTMLElement>()

/// Tracks the captain-relative "distance from bottom" snapshotted
/// right before vue-query starts a backward fetch. The post-fetch
/// `watch` on `data.pages.length` reads this, restores
/// `scrollTop = scrollHeight - distance`, then clears it. Holding the
/// distance in a ref (vs locally scoped to a fetch invocation) makes
/// the restore robust to vue-query's internal retries — every
/// successful page that lands while a distance is captured uses the
/// same anchor, so successive backward pages compose smoothly.
const restorationDistance = ref<number | undefined>(undefined)

/// `true` from the moment we ask vue-query for an older page until
/// the scroll-restore has finished. Gates eviction (we never trim
/// during a backward fetch) and signals the loading chip. Driven by
/// vue-query's own `isFetchingNextPage` plus the post-fetch nextTick
/// settle, so a stuck/long fetch can't leave it stale — vue-query
/// owns the lifecycle.
const isRestoringScroll = ref(false)

/// Captures the captain's distance-from-bottom BEFORE asking for
/// older content. The browser preserves what's pinned at the bottom
/// of the scroll viewport (`scrollHeight` grows + `scrollTop` grows
/// by the same delta when items prepend), so `scrollHeight -
/// scrollTop` is invariant across the prepend. Re-applying it after
/// the DOM update keeps the captain's reading line at the exact same
/// visual position.
function captureBeforeBackwardFetch(): void {
  const el = scrollEl.value

  if (!el) {
    return
  }

  if (restorationDistance.value !== undefined) {
    // Another fetch is already in flight; vue-query is deduping for
    // us, so we just keep the original anchor.
    return
  }
  restorationDistance.value = el.scrollHeight - el.scrollTop
  isRestoringScroll.value = true
  // Synchronously release stick — the captain is reading older
  // content, and any in-flight `useStickToBottom` rAF would
  // otherwise scroll-to-bottom mid-restore and clobber the anchor.
  releaseStickAndMark()
}

/// Restore the captain's reading position after a backward fetch
/// resolved + Vue flushed the new DOM. Two `nextTick`s give nested
/// child watchers (`<Turn>`'s elapsed-timer, `<StreamCard>`'s
/// markdown render) time to settle before we read `scrollHeight`.
async function restoreAfterBackwardFetch(): Promise<void> {
  await nextTick()
  await nextTick()
  const el = scrollEl.value
  const distance = restorationDistance.value

  if (el && distance !== undefined) {
    const target = el.scrollHeight - distance

    if (Math.abs(el.scrollTop - target) > 1) {
      el.scrollTop = target
    }
  }
  restorationDistance.value = undefined
  isRestoringScroll.value = false
}

// ── Backward fetch trigger (intersection-observer driven) ──────────
//
// `useIntersectionObserver` against the top-of-list sentinel calls
// `viewport.fetchNextPage` exactly once per visibility transition.
// vue-query dedupes concurrent calls internally — its
// `isFetchingNextPage` flag is the authoritative gate. The captain
// reported scrollTop-threshold + manual loadingEarlier ref +
// watchdog (the prior shape) flaked under mobile inertia + remote
// WS latency; sentinel + vue-query's built-in lifecycle is the
// canonical TanStack pattern + cuts ~80 lines of state-management
// scaffolding.
useIntersectionObserver(
  topSentinel,
  ([entry]) => {
    if (!entry?.isIntersecting) {
      return
    }

    if (!viewport.hasNextPage.value) {
      return
    }

    if (viewport.isFetchingNextPage.value) {
      return
    }
    captureBeforeBackwardFetch()
    void viewport.fetchNextPage().catch((err: unknown) => {
      log.warn('chat-viewport: backward fetch rejected', undefined, err)
    })
  },
  {
    // Pre-fetch BEFORE the captain visually hits the sentinel.
    // Expanding the root margin upward by `LOAD_MORE_SENTINEL_MARGIN_PX`
    // lights the observer when the sentinel is still that many px
    // outside the viewport's top edge — older content lands while
    // the captain is still scrolling, no perceptible pause.
    rootMargin: `${LOAD_MORE_SENTINEL_MARGIN_PX}px 0px 0px 0px`,
    // 0 = the moment any part of the sentinel enters the (expanded)
    // root. Sentinel is 0-height + 1px wide so this is
    // effectively a point-trigger.
    threshold: 0
  }
)

// Watch vue-query's `isFetchingNextPage` for the true→false
// transition. That's the moment the fetched page has been
// `setQueryData`-applied — restore the captain's scroll anchor.
// Conditioned on `restorationDistance` having been captured (we
// don't try to restore when the fetch wasn't sentinel-driven, e.g.
// the initial fetch on mount).
watch(
  () => viewport.isFetchingNextPage.value,
  (next, prev) => {
    if (prev && !next && restorationDistance.value !== undefined) {
      void restoreAfterBackwardFetch()
    }
  }
)

// ── Eviction trigger ────────────────────────────────────────────────
//
// Backward pagination lives on the intersection-observer wired
// below; this handler only drives the page-eviction half: when the
// captain is within ~one viewport of the bottom AND the cache
// exceeds `MAX_PAGES_KEPT`, drop the trailing pages. Wider than
// `useStickToBottom`'s strict 64px so eviction fires the moment the
// captain returns to the live area, not only at the absolute foot.
// The composable's eviction is idempotent — we can safely call it
// on every scroll tick that satisfies the near-bottom test; it's a
// no-op when the cache is already in budget.
function onScroll(): void {
  const el = scrollEl.value

  if (!el) {
    return
  }

  // Never trim during a backward fetch + scroll-restore: trimming
  // the tail (oldest pages) while a head/anchor restore is in
  // flight changes `scrollHeight` mid-restore and the anchor lands
  // off by the evicted pages' aggregate height. Eviction can wait
  // until the next scroll tick after the restore completes.
  if (isRestoringScroll.value) {
    return
  }

  if (!hasUserScrolled.value) {
    return
  }

  // Eviction trigger — within one viewport of the bottom. Defer
  // the actual cache mutation to the next animation frame so the
  // DOM patch doesn't race the in-flight scroll gesture. When
  // eviction fires synchronously inside the scroll handler, the
  // resulting microtask removes nodes from the TOP of the DOM
  // while the browser is mid-gesture — scroll-anchoring can miss
  // (the anchor element itself may be evicted), and concurrent
  // `useStickToBottom` observers queue an rAF off the same DOM
  // mutation, doubling the disruption. rAF moves the mutation
  // out of the scroll-event task, after the browser has finished
  // processing the current tick. `evictExtraPages` is idempotent
  // — repeated rAF wrappers within a single gesture all observe
  // the same cache state; at most one mutates, subsequent calls
  // are within-budget no-ops.
  const distanceFromBottom = el.scrollHeight - el.scrollTop - el.clientHeight

  if (distanceFromBottom <= el.clientHeight) {
    // Capture whether the captain was at the live tail BEFORE
    // eviction. Eviction shrinks `scrollHeight` from the TOP — the
    // browser's `overflow-anchor` heuristic picks SOMETHING to keep
    // visually stable, but the choice is opaque and the captain
    // reported "jump to random places" when the anchor lands
    // somewhere unexpected (the chevron-click path at `goToBottom`
    // already does evict-then-nextTick-then-scroll for this exact
    // reason; the auto-eviction path didn't). Mirror that pattern
    // so the captain who was stuck-at-bottom lands back at the new
    // bottom after the DOM shrinks, instead of wherever scroll-
    // anchoring left them.
    const wasStuck = stuck.value

    requestAnimationFrame(() => {
      viewport.evictExtraPages()

      if (wasStuck) {
        void nextTick(() => {
          scrollToBottom()
        })
      }
    })
  }
}

const liveBlockIdx = computed<number>(() => {
  const open = openTurnId.value

  if (!open) {
    return -1
  }

  return blocks.value.findIndex((b) => b.turnId === open)
})

const liveNow = useNow({ interval: 1000 })

function liveNowMs(): number {
  return liveNow.value.getTime()
}

const turnDurationLabels = computed<Map<string, string>>(() => {
  const out = new Map<string, string>()
  const now = liveNowMs()

  for (const t of turnRecords.value) {
    if (typeof t.startedAtMs !== 'number' || t.startedAtMs === 0) {
      continue
    }
    const end = typeof t.endedAtMs === 'number' ? t.endedAtMs : now
    const elapsed = Math.max(0, end - t.startedAtMs)

    if (!Number.isFinite(elapsed)) {
      continue
    }
    out.set(t.id, formatDuration(elapsed))
  }

  return out
})

/// Latest `updatedAt` across every entry in a block — turn / stream
/// / tool. The `tryMergeIntoExisting` path mutates the existing
/// entry in-place when streaming chunks arrive (`prev.entry.turn.text
/// += chunk; prev.entry.turn.updatedAt = it.seq`), so a v-memo dep
/// based purely on array LENGTH wouldn't change per chunk and the
/// live row's text would freeze. The latest `updatedAt` advances on
/// every merge — for the live block it ticks per chunk; for history
/// blocks it stays stable post-turn-end so v-memo skips render.
interface UpdatedAtBlock {
  turnEntries: { turn: { updatedAt?: number } }[]
  streamEntries: { item: { updatedAt?: number } }[]
  toolCalls: { call: { updatedAt?: number } }[]
  thoughts: { call: { updatedAt?: number } }[]
}

function latestUpdatedAt(block: UpdatedAtBlock): number {
  let max = 0

  for (const t of block.turnEntries) {
    const u = t.turn.updatedAt ?? 0

    if (u > max) {
      max = u
    }
  }

  for (const s of block.streamEntries) {
    const u = s.item.updatedAt ?? 0

    if (u > max) {
      max = u
    }
  }

  for (const tc of block.toolCalls) {
    const u = tc.call.updatedAt ?? 0

    if (u > max) {
      max = u
    }
  }

  for (const th of block.thoughts) {
    const u = th.call.updatedAt ?? 0

    if (u > max) {
      max = u
    }
  }

  return max
}

function elapsedFor(turnId?: string): string | undefined {
  if (!turnId) {
    return undefined
  }

  return turnDurationLabels.value.get(turnId)
}

function usageFor(turnId?: string) {
  if (!turnId) {
    return undefined
  }

  return turnRecords.value.find((rec) => rec.id === turnId)?.usage
}

interface ThinkingElapsedBlock {
  turnId?: string
  thoughts: { call: { startedAtMs: number; completedAtMs?: number } }[]
}

function thinkingElapsedFor(block: ThinkingElapsedBlock): string | undefined {
  const now = liveNowMs()
  let totalMs = 0
  let hasSignal = false

  if (block.turnId !== undefined) {
    const turn = turnRecords.value.find((rec) => rec.id === block.turnId)

    if (turn !== undefined) {
      const closed = typeof turn.thinkingMs === 'number' ? turn.thinkingMs : 0
      const open = typeof turn.thinkingOpenAtMs === 'number' ? Math.max(0, now - turn.thinkingOpenAtMs) : 0
      const stream = closed + open

      if (stream > 0 || turn.thinkingOpenAtMs !== undefined) {
        totalMs += stream
        hasSignal = true
      }
    }
  }

  for (const entry of block.thoughts) {
    const s = entry.call.startedAtMs

    if (typeof s !== 'number' || s <= 0) {
      continue
    }
    const c = entry.call.completedAtMs
    const end = typeof c === 'number' ? c : now

    totalMs += Math.max(0, end - s)
    hasSignal = true
  }

  if (!hasSignal || !Number.isFinite(totalMs)) {
    return undefined
  }

  return formatDuration(totalMs)
}

function hasThinkingSignal(block: { turnId?: string; thoughts: { call: { startedAtMs: number } }[] }): boolean {
  if (block.turnId !== undefined) {
    const turn = turnRecords.value.find((rec) => rec.id === block.turnId)

    if (turn !== undefined) {
      const closed = typeof turn.thinkingMs === 'number' ? turn.thinkingMs : 0

      if (closed > 0 || turn.thinkingOpenAtMs !== undefined) {
        return true
      }
    }
  }

  for (const entry of block.thoughts) {
    if (typeof entry.call.startedAtMs === 'number' && entry.call.startedAtMs > 0) {
      return true
    }
  }

  return false
}

function thoughtText(call: { title?: string; content: { type?: string; text?: string }[]; rawInput?: Record<string, unknown> }): string {
  const parts: string[] = []
  const summary = call.title?.trim()

  if (summary && summary.length > 0) {
    parts.push(`**${summary}**`)
  }

  for (const c of call.content ?? []) {
    if (typeof c.text === 'string' && c.text.trim().length > 0) {
      parts.push(c.text)
    }
  }

  if (parts.length === 0 && call.rawInput) {
    const raw = call.rawInput.thought ?? call.rawInput.text ?? call.rawInput.description

    if (typeof raw === 'string') {
      parts.push(raw)
    }
  }

  return parts.join('\n\n')
}

function combinedThoughtText(block: {
  thoughts: { createdAt: number; call: WireToolCall }[]
  streamEntries: { createdAt: number; item: { kind: StreamItemKind; text?: string } }[]
}): string {
  const merged: { createdAt: number; text: string }[] = []

  for (const entry of block.thoughts) {
    const text = thoughtText(entry.call)

    if (text.length > 0) {
      merged.push({ createdAt: entry.createdAt, text })
    }
  }

  for (const entry of block.streamEntries) {
    if (entry.item.kind !== StreamItemKind.Thought) {
      continue
    }
    const text = entry.item.text ?? ''

    if (text.length > 0) {
      merged.push({ createdAt: entry.createdAt, text })
    }
  }
  merged.sort((a, b) => a.createdAt - b.createdAt)

  return merged.map((m) => m.text).join('\n\n')
}

function systemPromptLabel(files: readonly string[]): string {
  if (files.length === 0) {
    return 'attached'
  }
  const baseNames = files.map((f) => f.split('/').pop() ?? f)

  if (baseNames.length <= 3) {
    return baseNames.join(', ')
  }

  return `${baseNames.slice(0, 3).join(', ')} +${baseNames.length - 3} more`
}

function mapPlanStatus(raw?: string): PlanStatus {
  switch (raw) {
    case 'completed':
      return PlanStatus.Completed

    case 'in_progress':
      return PlanStatus.InProgress

    default:
      return PlanStatus.Pending
  }
}

function mapPlanItems(entries: PlanEntry[]): PlanItem[] {
  return entries.map((e) => ({ status: mapPlanStatus(e.status), text: e.content ?? '' }))
}

function terminalIdForCall(call: { rawInput?: Record<string, unknown> }): string | undefined {
  const raw = call.rawInput

  if (!raw) {
    return undefined
  }
  const candidate = raw.terminal_id ?? raw.terminalId

  return typeof candidate === 'string' && candidate.length > 0 ? candidate : undefined
}

const isEmpty = computed(() => blocks.value.length === 0)

defineExpose({ scrollEl })
</script>

<template>
  <div class="chat-viewport-root">
    <div ref="scrollEl" class="chat-transcript" data-testid="chat-transcript" :data-instance-id="instanceId ?? ''" @scroll="onScroll">
      <Loading v-if="props.restoring" mode="scoped" status="restoring session — replaying transcript" />

      <slot v-if="isEmpty" name="empty" />

      <template v-else>
        <!-- Top sentinel — 0-height marker the IntersectionObserver
           watches. When it enters the captain's expanded viewport
           (rootMargin LOAD_MORE_SENTINEL_MARGIN_PX above the actual
           top), `viewport.fetchNextPage` fires. Rendered ONLY when
           older pages still exist so the observer naturally
           short-circuits at the end of history. -->
        <div v-if="viewport.hasNextPage.value" ref="topSentinel" class="chat-top-sentinel" aria-hidden="true" data-testid="chat-top-sentinel" />

        <!-- Loading chip — sticky-pinned to the visible top edge so
           it stays in view regardless of scroll position during the
           fetch. Combined gate: vue-query's own `isFetchingNextPage`
           (the fetch is in flight) OR `isRestoringScroll` (vue-query
           returned but we're still settling the captain's scroll
           anchor across the next two Vue ticks). -->
        <div v-if="viewport.isFetchingNextPage.value || isRestoringScroll" class="chat-load-chip animate-pulse" data-testid="chat-load-chip">loading earlier…</div>

        <!-- Plain v-for over `blocks`, with `v-memo` short-circuiting
           re-renders for history rows. Live row keeps re-rendering
           every chunk because `latestUpdatedAt(block)` advances per
           streaming merge (every chunk bumps the corresponding
           entry's `updatedAt` to the new seq). History rows have a
           stable `latestUpdatedAt` post-turn-end so v-memo skips
           their VNode walk entirely under streaming.
           Deps cover: turn identity, role, content-shape counts,
           per-chunk freshness via `latestUpdatedAt`, live flag,
           elapsed/usage labels. -->
        <Turn
          v-for="(block, blockIdx) in blocks"
          :key="block.groupKey"
          v-memo="[
            block.groupKey,
            block.role,
            block.turnEntries.length,
            block.toolCalls.length,
            block.streamEntries.length,
            latestUpdatedAt(block),
            blockIdx === liveBlockIdx,
            elapsedFor(block.turnId),
            usageFor(block.turnId)
          ]"
          :role="block.role"
          :live="blockIdx === liveBlockIdx"
          :elapsed="elapsedFor(block.turnId)"
          :usage="usageFor(block.turnId)"
        >
          <StreamCard
            v-if="combinedThoughtText(block).length > 0 || hasThinkingSignal(block)"
            :kind="StreamKind.Thinking"
            :active="blockIdx === liveBlockIdx"
            label="thought"
            :elapsed="thinkingElapsedFor(block)"
            :text="combinedThoughtText(block).length > 0 ? combinedThoughtText(block) : undefined"
          />
          <template v-for="entry in block.streamEntries" :key="`stream-${entry.createdAt}`">
            <StreamCard
              v-if="entry.item.kind === StreamItemKind.Plan"
              :kind="StreamKind.Planning"
              :active="blockIdx === liveBlockIdx"
              label="plan"
              :items="mapPlanItems(entry.item.entries)"
              :stats="entry.item.stats"
            />
            <ChangeBanner
              v-else-if="entry.item.kind === StreamItemKind.ModeChange"
              kind="mode"
              :to="entry.item.name ?? entry.item.modeId"
              :from="entry.item.prevName ?? entry.item.prevModeId"
            />
            <ChangeBanner
              v-else-if="entry.item.kind === StreamItemKind.ModelChange"
              kind="model"
              :to="entry.item.name ?? entry.item.modelId"
              :from="entry.item.prevName ?? entry.item.prevModelId"
            />
            <ChangeBanner
              v-else-if="entry.item.kind === StreamItemKind.ConfigOptionChange"
              :kind="entry.item.categoryId"
              :to="entry.item.name ?? entry.item.value"
              :from="entry.item.prevName ?? entry.item.prevValue"
            />
            <ChangeBanner v-else-if="entry.item.kind === StreamItemKind.SystemPromptInjected" kind="system prompt" :to="systemPromptLabel(entry.item.files)" />
          </template>

          <ToolChips v-if="block.toolCalls.length > 0" :views="block.toolCalls.map((t) => format(t.call, adapterForActive))" />

          <template v-for="entry in block.toolCalls" :key="`term-${entry.call.toolCallId}`">
            <TerminalCard v-if="terminalIdForCall(entry.call)" :terminal-id="terminalIdForCall(entry.call) ?? ''" :instance-id="instanceId" @cancel="emit('cancel')" />
          </template>

          <template v-for="entry in block.turnEntries" :key="`turn-${entry.createdAt}`">
            <Body v-if="entry.turn.role === TurnRole.Agent" :role="Role.Assistant" :text="entry.turn.text" markdown />
            <template v-else>
              <Body :role="Role.User" :text="entry.turn.text" markdown />
              <Attachments
                v-if="entry.turn.attachments && entry.turn.attachments.length > 0"
                :attachments="entry.turn.attachments"
                @open="(att) => emit('attachment-open', att)"
              />
            </template>
          </template>
        </Turn>
      </template>
    </div>

    <!-- Floating scroll-to-bottom chevron. Lives inside the viewport
         (anchored to the chat scroller's bottom-right) so it's not
         coupled to whatever sits below the viewport (composer, queue,
         permission stack). Visible only when the captain has scrolled
         away from the bottom — `stuck` flips false the moment they
         move >64px above the foot. Click jumps to the live area AND
         immediately drops any extra pages the captain accumulated
         while scrolling up. -->
    <button v-if="!stuck" type="button" class="scroll-to-bottom" data-testid="scroll-to-bottom" aria-label="Scroll to latest" @click="goToBottom">
      <FaIcon :icon="faChevronDown" />
    </button>
  </div>
</template>

<style scoped>
@reference '../../assets/styles.css';

/* Wrapper takes the flex-1 + min-h-0 the parent expects of the
 * Viewport root, so the inner scroller can fill while leaving room
 * for the absolute-positioned floating chevron without scrolling
 * with the content. */
.chat-viewport-root {
  @apply relative flex min-h-0 flex-1 flex-col;
}

.chat-transcript {
  @apply flex min-h-0 flex-1 flex-col overflow-y-auto;
  position: relative;
  padding: 0 0.875rem 0 0.25rem;
  /* Disable browser-native scroll anchoring. Chrome / WebKit default
   * to `overflow-anchor: auto` on scrollable containers, which the
   * browser uses to compensate scrollTop automatically when content
   * is added above the visible region. That fights the manual
   * compensation `triggerBackwardFetch` does (capture scrollHeight -
   * scrollTop before fetch, restore after), causing double-shift
   * and a visible jump. JS owns the anchoring — disable the native
   * path so the two don't compose. */
  overflow-anchor: none;
}

.chat-top-sentinel {
  /* 0-height, full-width marker the IntersectionObserver watches.
   * `pointer-events: none` so it never intercepts clicks; absolutely
   * no visual presence. */
  flex-shrink: 0;
  height: 1px;
  width: 100%;
  pointer-events: none;
}

.chat-load-chip {
  @apply rounded text-[0.7rem];
  /* Sticky to the top of the scroll viewport so the chip stays
   * visible while older content loads. `top: 0.5rem` matches the
   * margin so the chip's resting position looks identical to the
   * pre-sticky version when scrolled to the head; while scrolled
   * down it rides the top edge of the visible area. `z-index: 1`
   * keeps it above the (non-positioned) <Turn> rows. */
  position: sticky;
  top: 0.5rem;
  z-index: 1;
  margin: 0.5rem auto 0.25rem;
  padding: 0.125rem 0.5rem;
  background-color: var(--theme-surface-alt);
  color: var(--theme-fg-dim);
  border: 1px solid var(--theme-border-soft);
  align-self: center;
}

/* Floating chevron — bottom-right of the chat surface. Sits above
 * the inner scroller so it doesn't move with content. Compact so
 * it doesn't hog vertical space on mobile; rem-based so the
 * mobile root-bump still scales it. */
.scroll-to-bottom {
  position: absolute;
  bottom: 0.625rem;
  right: 0.625rem;
  width: 1.5rem;
  height: 1.5rem;
  display: flex;
  align-items: center;
  justify-content: center;
  border-radius: 9999px;
  background-color: var(--theme-surface-alt);
  color: var(--theme-fg-dim);
  border: 1px solid var(--theme-border-soft);
  cursor: pointer;
  font-size: 0.7rem;
  transition:
    background-color 120ms ease,
    color 120ms ease,
    transform 120ms ease;
  z-index: 5;
}

.scroll-to-bottom:hover {
  background-color: var(--theme-surface);
  color: var(--theme-fg);
}

.scroll-to-bottom:active {
  transform: translateY(1px);
}
</style>
