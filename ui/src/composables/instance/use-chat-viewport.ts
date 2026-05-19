/**
 * Surface composable for the virtualized chat body (Phase C1).
 *
 * Combines `useInstanceChatInfiniteQuery` (the data layer) with the
 * flattened oldest-first items view + `latestSeq` cursor consumers
 * read. Backward pagination still rides the underlying
 * `useInfiniteQuery` handle; the cache holds every fetched page for
 * the lifetime of the instance.
 *
 * **Note on page eviction**: an earlier shape (Phase C1) trimmed the
 * cache to the newest `MAX_PAGES_KEPT` pages whenever the captain was
 * near the foot. It was removed in this PR — under rapid
 * session-load replay the eviction raced the patcher's
 * `setQueryData` mutations on the same key and silently dropped
 * replayed items. The daemon-side ring buffer already bounds the
 * captain's memory exposure; client-side trimming was protecting
 * against a problem that didn't materialise.
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
  //
  // Seq dedup as defense in depth: the daemon's `before` cursor is
  // exclusive (`mirror.rs::chat_snapshot` filters `seq >= cursor`),
  // so no overlap is expected today. But the patcher mutates page[0]
  // independently of backward fetches, and `setQueryData` can race
  // a backward fetch in flight — if a live event for a seq that's
  // also returned by the racing fetch lands in both pages, the
  // projector would render the item twice. A Set-keyed seq filter
  // is O(N) on a list capped at ~150 rows, basically free, and
  // closes both the cross-page-boundary race AND any future
  // off-by-one bug in cursor handling.
  const items = computed<SeqTranscriptItem[]>(() => {
    const data = query.data.value as PatchableInfiniteData | undefined

    if (!data || data.pages.length === 0) {
      return []
    }
    const out: SeqTranscriptItem[] = []
    const seen = new Set<number>()

    for (let p = data.pages.length - 1; p >= 0; p -= 1) {
      const page = data.pages[p]

      if (!page) {
        continue
      }

      for (const it of page.items) {
        if (seen.has(it.seq)) {
          continue
        }
        seen.add(it.seq)
        out.push(it)
      }
    }

    return out
  })

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
    fetchNextPage
  }
}
