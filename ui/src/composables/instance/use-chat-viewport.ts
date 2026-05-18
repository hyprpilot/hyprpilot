/**
 * Surface composable for the virtualized chat body (Phase C1).
 *
 * Combines `useInstanceChatInfiniteQuery` (the data layer) with the
 * two concerns the body view needs on top:
 *
 * 1. **Page-trim policy** — when the viewport is at the bottom AND
 *    the cache holds more than `MAX_PAGES_KEPT` pages, drop pages
 *    `0..(N-MAX_PAGES_KEPT)`. Keeps memory bounded under long
 *    sessions without user-visible truncation: the daemon always
 *    serves older pages on backward scroll. Triggered from the
 *    body view's scroll handler whenever the captain is within
 *    ~one viewport of bottom (wider than `useStickToBottom`'s
 *    stick threshold so cleanup is prompt without disturbing
 *    read-history flow). The body view schedules the mutation via
 *    `requestAnimationFrame` so the cache write lands outside the
 *    scroll-event task — see Viewport.vue's onScroll for the
 *    timing rationale.
 *
 * 2. **Flattened oldest-first items view** for the snapshot
 *    projector + a `latestSeq` cursor for diagnostic consumers.
 *
 * Live patching (`acp:transcript` + `acp:permission-resolved`) lives
 * in the module-level singleton at `./transcript-patcher.ts`. The
 * earlier per-composable IIFE only wired itself when this composable
 * mounted, which on remote landed after the WS auto-subscribed to
 * events — every event arriving in that gap was dropped on the floor
 * at the remote-bridge dispatcher (no listener registered for the
 * event name yet). `startTranscriptPatcher` runs from `main.ts`
 * before the first boot RPC fires, so the dispatcher's
 * `eventListeners.get('acp:transcript')` is populated by the time
 * the daemon pushes its first frame.
 *
 * The exposed shape is a thin pass-through over the underlying
 * `useInfiniteQuery` handle plus a flattened `items` ref (oldest-
 * first) and a `latestSeq` cursor for downstream consumers.
 */

import { useQueryClient, type InfiniteData } from '@tanstack/vue-query'
import { computed, type ComputedRef, type Ref } from 'vue'

import { useInstanceChatInfiniteQuery, type UseInstanceChatInfiniteQueryReturn } from './use-instance-chat-infinite-query'
import { type InstanceId } from '../chrome/use-active-instance'
import { type ChatSnapshot, type SeqTranscriptItem } from '@ipc'
import { log } from '@lib'

/**
 * Cache pages retained when stuck-at-bottom. Each page is sized to
 * cover the viewport (see `viewportPageSize` below), so 3 pages = 3
 * viewports of scrollback in DOM at peak.
 */
export const MAX_PAGES_KEPT = 3

/**
 * Fallback row-height estimate in CSS px. Used only when no items
 * have rendered yet (initial fetch races mount). Once any items
 * are in the DOM, [`measuredPageSize`] computes the real
 * px-per-item from `scrollHeight / itemCount` and the fallback is
 * irrelevant.
 */
const ROW_HEIGHT_ESTIMATE_PX = 96

/** Lower clamp so even a tiny viewport gets a useful chunk. */
const MIN_PAGE_SIZE = 20

/**
 * **Initial-fetch** page size — used only when no items have
 * rendered yet. Falls back to a 96-px heuristic. Once the cache
 * has any items, [`measuredPageSize`] takes over with a
 * data-driven estimate.
 */
export function viewportPageSize(scrollEl: Ref<HTMLElement | undefined>): number {
  const el = scrollEl.value
  const h = el?.clientHeight ?? 0

  if (h <= 0) {
    return MIN_PAGE_SIZE
  }

  return Math.max(MIN_PAGE_SIZE, Math.ceil(h / ROW_HEIGHT_ESTIMATE_PX))
}

/**
 * **Measured** page size — uses the real DOM dimensions to
 * compute "items per viewport" once the chat has rendered any
 * content.
 *
 * Math: if `itemCount` items occupy `scrollHeight` px of rendered
 * content, the average item is `scrollHeight / itemCount` px tall.
 * One viewport (`clientHeight`) holds
 * `clientHeight / pxPerItem = clientHeight × itemCount /
 * scrollHeight` items.
 *
 * Self-corrects: as the captain scrolls and more items render,
 * `scrollHeight` grows, the estimate refines. Wildly tall agent
 * replies push the next fetch smaller; short user-prompt-only
 * turns push it bigger. No fixed-px guess required.
 *
 * Falls back to [`viewportPageSize`] when no items are rendered
 * yet (the very first fetch).
 */
export function measuredPageSize(scrollEl: Ref<HTMLElement | undefined>, itemCount: number): number {
  const el = scrollEl.value
  const h = el?.clientHeight ?? 0
  const total = el?.scrollHeight ?? 0

  if (itemCount <= 0 || h <= 0 || total <= 0) {
    return viewportPageSize(scrollEl)
  }

  const itemsPerViewport = (itemCount * h) / total

  return Math.max(MIN_PAGE_SIZE, Math.ceil(itemsPerViewport))
}

export interface UseChatViewportApi {
  /** Flattened, oldest-first transcript items across every cached page. */
  items: ComputedRef<SeqTranscriptItem[]>
  /** Latest known `seq` cursor — incremented locally on live patches. */
  latestSeq: ComputedRef<number | undefined>
  /** Initial-load gate; `true` while the first page is in flight. */
  isInitialLoading: ComputedRef<boolean>
  /** Backward-pagination in-flight gate. */
  isFetchingNextPage: ComputedRef<boolean>
  /** `true` while older pages remain on the daemon. */
  hasNextPage: ComputedRef<boolean>
  /** Trigger a backward fetch; safe to call when in-flight (TanStack dedupes). */
  fetchNextPage: () => Promise<unknown>
  /**
   * Drop pages older than `MAX_PAGES_KEPT`. Idempotent — if the cache
   * is already within budget it's a no-op. The body view calls this
   * whenever the captain is "in the live area" (within ~one viewport
   * of bottom) so a re-entry from history reading triggers cleanup
   * without waiting for the strict stuck-at-bottom threshold the auto-
   * scroll uses. Newest pages are kept; oldest get dropped — the
   * daemon serves them again on backward scroll.
   */
  evictExtraPages: () => void
}

interface PatchableInfiniteData extends InfiniteData<ChatSnapshot, number | undefined> {
  pages: ChatSnapshot[]
  pageParams: (number | undefined)[]
}

export interface UseChatViewportOptions {
  /**
   * The chat surface's scroll container. When provided, the
   * fetch page size is computed dynamically from
   * `clientHeight` so each page covers ~one viewport. Without
   * this the query falls back to `DEFAULT_CHAT_LIMIT` (50) —
   * fine for prefetch / brim-sync paths that don't have a
   * viewport yet.
   */
  scrollEl?: Ref<HTMLElement | undefined>
}

export function useChatViewport(instanceId: ComputedRef<InstanceId | undefined>, opts: UseChatViewportOptions = {}): UseChatViewportApi {
  const queryClient = useQueryClient()

  // Limit getter — re-evaluated on every backward fetch. Reads the
  // CURRENT cache item count + scrollEl dimensions and asks
  // `measuredPageSize` for "items per viewport". Self-corrects: the
  // first fetch lands on the heuristic fallback (no items rendered
  // yet); every subsequent fetch is sized off real DOM extent, so a
  // chat with mostly-short user prompts pulls bigger pages while
  // one with massive agent replies pulls smaller ones.
  const limitGetter = opts.scrollEl
    ? (): number => {
      const id = instanceId.value
      const data = id !== undefined ? (queryClient.getQueryData(['snapshot-chat', id]) as PatchableInfiniteData | undefined) : undefined
      const itemCount = data?.pages.reduce((sum, p) => sum + (p?.items.length ?? 0), 0) ?? 0

      return measuredPageSize(opts.scrollEl as Ref<HTMLElement | undefined>, itemCount)
    }
    : undefined

  const query: UseInstanceChatInfiniteQueryReturn = useInstanceChatInfiniteQuery(instanceId, { limit: limitGetter })

  // Local seq counter so live items get monotonically-increasing
  // ordinals. We don't try to mimic the daemon's actual `seq` — the
  // value is only used for backward pagination cursors, and live
  // items always land at the head (never used as a cursor).
  // Initialised lazily off the first non-empty page's `latestSeq`.
  const localSeq: ComputedRef<number | undefined> = computed(() => {
    const data = query.data.value as PatchableInfiniteData | undefined

    if (!data || data.pages.length === 0) {
      return undefined
    }

    // Walk pages newest-first looking for the highest seq we've seen.
    // The first page (index 0) is the newest by the daemon contract.
    for (const page of data.pages) {
      const items = page.items

      if (items.length === 0) {
        continue
      }
      const last = items[items.length - 1]

      if (last && last.seq > (page.latestSeq ?? -1)) {
        return last.seq
      }

      if (page.latestSeq !== undefined) {
        return page.latestSeq
      }
    }

    return undefined
  })

  // Flattened, oldest-first view. The daemon serves the newest page
  // first (page 0); within a page items are oldest-first. To produce
  // an oldest-first stream we walk pages from last to first, then
  // each page's items in their natural order.
  const items = computed<SeqTranscriptItem[]>(() => {
    const data = query.data.value as PatchableInfiniteData | undefined

    if (!data || data.pages.length === 0) {
      return []
    }
    const out: SeqTranscriptItem[] = []

    for (let p = data.pages.length - 1; p >= 0; p -= 1) {
      const page = data.pages[p]

      if (!page) {
        continue
      }

      for (const it of page.items) {
        out.push(it)
      }
    }

    return out
  })

  // Page-trim policy. Idempotent: drop pages older than the
  // newest `MAX_PAGES_KEPT` whenever the caller signals "the
  // captain is in the live area, dropping older pages won't yank
  // anything they're reading." The body view calls this from its
  // scroll handler when the captain is within ~one viewport of
  // bottom — wider than `useStickToBottom`'s stick threshold (which
  // gates the auto-scroll behaviour). The wider eviction trigger
  // means the cache cleans up promptly when the captain returns
  // to live, instead of waiting for the strict stuck-at-bottom
  // condition that the auto-scroll uses.
  function evictExtraPages(): void {
    const id = instanceId.value

    if (id === undefined) {
      return
    }
    queryClient.setQueryData<PatchableInfiniteData>(['snapshot-chat', id], (old) => {
      if (!old) {
        log.trace('snapshot.page-trim.skipped-no-cache', { instanceId: id })

        return old
      }

      if (old.pages.length <= MAX_PAGES_KEPT) {
        log.trace('snapshot.page-trim.skipped-within-budget', {
          instanceId: id,
          pages: old.pages.length,
          max: MAX_PAGES_KEPT
        })

        return old
      }
      // Page 0 is newest. We keep the newest `MAX_PAGES_KEPT` pages
      // and drop the rest. The dropped pages live on the daemon — a
      // backward scroll re-fetches them.
      const keptPages = old.pages.slice(0, MAX_PAGES_KEPT)
      const keptParams = old.pageParams.slice(0, MAX_PAGES_KEPT)

      log.trace('snapshot.page-trim.evicted', {
        instanceId: id,
        before: old.pages.length,
        after: keptPages.length
      })

      return {
        ...old,
        pages: keptPages,
        pageParams: keptParams
      }
    })
  }

  // Wrapped fetchNextPage exposing a uniform Promise<unknown> shape
  // — TanStack's typed return is hard to thread through the API
  // surface and consumers don't read it.
  async function fetchNextPage(): Promise<unknown> {
    const id = instanceId.value
    const data = query.data.value as PatchableInfiniteData | undefined
    const before = data?.pages[data.pages.length - 1]?.oldestSeq

    log.trace('snapshot.fetch-older.start', {
      instanceId: id,
      before,
      pages: data?.pages.length ?? 0
    })
    const result = await query.fetchNextPage()

    log.trace('snapshot.fetch-older.done', {
      instanceId: id,
      pages: (query.data.value as PatchableInfiniteData | undefined)?.pages.length ?? 0
    })

    return result
  }

  // Stable computeds over the underlying refs — exposing the raw
  // refs would leak the TanStack handle's identity (re-creating the
  // query swaps the ref, which can desync subscriber bindings).
  const isInitialLoading = computed(() => query.isFetching.value && (query.data.value === undefined || (query.data.value as PatchableInfiniteData).pages.length === 0))
  const isFetchingNextPage = computed(() => query.isFetchingNextPage.value)
  const hasNextPage = computed(() => query.hasNextPage.value)

  return {
    items,
    latestSeq: localSeq,
    isInitialLoading,
    isFetchingNextPage,
    hasNextPage,
    fetchNextPage,
    evictExtraPages
  }
}
