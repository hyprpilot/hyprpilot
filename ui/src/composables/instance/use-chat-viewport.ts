/**
 * Surface composable for the virtualized chat body (Phase C1).
 *
 * Combines `useInstanceChatInfiniteQuery` (the data layer) with the
 * three concerns the body view needs on top:
 *
 * 1. **Live event patching** — incoming `acp:transcript` events (and
 *    sibling `acp:turn-started` / `acp:turn-ended`) land on the
 *    *latest page* of the cached infinite query via `setQueryData`.
 *    Without this, new chunks during streaming wouldn't appear until
 *    the page refetched. Tool-call updates merge with the existing
 *    item by `tool_call_id` instead of appending. The local `seq`
 *    counter starts at the daemon's `latestSeq` and increments on
 *    every patched item — UI doesn't know the daemon's actual `seq`,
 *    but it doesn't need to: the cursor is only used for backward
 *    pagination, and live events always land at the head.
 *
 * 2. **Permission-resolved cache invalidation** — `acp:permission-resolved`
 *    drops the matching entry from `MetaSnapshot.pendingPermissions`
 *    so a captain answering on the desktop clears the prompt on the
 *    remote (and vice versa).
 *
 * 3. **Page-trim policy** — when the viewport is at the bottom AND
 *    the cache holds more than `MAX_PAGES_KEPT` pages, drop pages
 *    `0..(N-MAX_PAGES_KEPT)`. Keeps memory bounded under long
 *    sessions without user-visible truncation: the daemon always
 *    serves older pages on backward scroll. Triggered by the
 *    `useStickToBottom` "stuck" signal so trimming only fires while
 *    the captain isn't reading older history.
 *
 * The exposed shape is a thin pass-through over the underlying
 * `useInfiniteQuery` handle plus a flattened `items` ref (oldest-
 * first) and a `latestSeq` cursor for downstream consumers.
 */

import { useQueryClient, type InfiniteData } from '@tanstack/vue-query'
import { computed, onUnmounted, watch, type ComputedRef, type Ref } from 'vue'

import { useInstanceChatInfiniteQuery, type UseInstanceChatInfiniteQueryReturn } from './use-instance-chat-infinite-query'
import { usePermissions } from './use-permissions'
import { type InstanceId } from '../chrome/use-active-instance'
import {
  listen,
  TauriEvent,
  TranscriptItemKind,
  type ChatSnapshot,
  type MetaSnapshot,
  type SeqTranscriptItem,
  type TranscriptEventPayload,
  type AcpPermissionResolvedPayload,
  type UnlistenFn
} from '@ipc'
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
  /** Unsubscribe the live-event listeners — wired automatically on unmount. */
  stop: () => void
}

interface PatchableInfiniteData extends InfiniteData<ChatSnapshot, number | undefined> {
  pages: ChatSnapshot[]
  pageParams: (number | undefined)[]
}

/**
 * Merge a `tool_call_update` event onto its existing tool-call
 * entry on the latest page. Returns `true` on merge.
 *
 * Wire shape: ACP `tool_call_update` carries the same `id` as the
 * original `tool_call`. The daemon's `tool_call_cache.merge`
 * concatenates `content` arrays (`acp/instance.rs`:
 * `running.content.extend(arr.iter().cloned())`) — the wire ships
 * per-delta `content`, so the UI must concat to reconstruct the
 * merged view that the daemon's formatter produces. The snapshot-
 * replay path in `snapshot-timeline.ts::mergeToolCall` does the
 * same; this helper keeps the live-patch path in lockstep.
 *
 * `turnId` preservation: the merged `SeqTranscriptItem` MUST keep
 * the existing `turnId` (the tool call's anchor to the active turn).
 * Dropping it here would orphan the call out of its turn block —
 * `timelineBlocksFromSnapshot` groups by `turnId`, so an undefined
 * turnId falls into a phantom "snapshot-assistant:N" block instead
 * of the `turn:X` block where the rest of the turn lives. That
 * cascades into perceived bugs: (1) tool pills with no Turn header
 * chips above them, (2) thought chunks that arrive after the tool
 * landing in a separate block from the thoughts that came before
 * (because the tool block sits between them).
 *
 * Single-shape items.length scan from the tail finds the matching
 * call regardless of initial vs update kind.
 */
function mergeToolCallUpdate(items: SeqTranscriptItem[], incoming: SeqTranscriptItem): boolean {
  const next = incoming.item

  if (next.kind !== TranscriptItemKind.ToolCallUpdate) {
    return false
  }
  const id = next.id

  for (let i = items.length - 1; i >= 0; i -= 1) {
    const existing = items[i]

    if (!existing) {
      continue
    }
    const ex = existing.item

    if ((ex.kind === TranscriptItemKind.ToolCall || ex.kind === TranscriptItemKind.ToolCallUpdate) && ex.id === id) {
      const merged: typeof ex = { ...ex }

      if (next.toolKind !== undefined) {
        merged.toolKind = next.toolKind
      }

      if (next.title !== undefined) {
        merged.title = next.title
      }

      if (next.state !== undefined) {
        merged.state = next.state
      }

      if (next.rawInput !== undefined) {
        merged.rawInput = next.rawInput
      }

      // Concat content — see fn header.
      if (next.content && next.content.length > 0) {
        merged.content = [...(ex.content ?? []), ...next.content]
      }
      merged.formatted = next.formatted
      merged.startedAtMs = next.startedAtMs

      if (next.completedAtMs !== undefined) {
        merged.completedAtMs = next.completedAtMs
      }
      // Preserve the existing turnId — the original `tool_call` event
      // stamped it; the wire `tool_call_update` may or may not echo
      // the turnId in its payload, but the merged item's anchor is
      // the original turn. Take the incoming turnId only as a fallback
      // (the existing was undefined).
      items[i] = {
        seq: existing.seq,
        turnId: existing.turnId ?? incoming.turnId,
        item: merged
      }

      return true
    }
  }

  return false
}

/**
 * Build a `SeqTranscriptItem` for a live transcript event. The local
 * `seq` is monotonically issued by the caller; the daemon-side seq
 * exists separately and isn't needed for live consumption.
 *
 * `turnId` MUST be carried through from the payload — the snapshot
 * path returns items already stamped with the active turn id, and
 * `timelineBlocksFromSnapshot` groups blocks by `turnId` to anchor
 * the Turn header chips (elapsed / usage / cost) to a useTurns
 * record. Dropping `turnId` here makes live-patched blocks render
 * with `block.turnId === undefined` → `usageFor(undefined)` returns
 * undefined → chips don't render even though the underlying turn
 * record DOES carry the usage reading.
 */
function liveItemFor(payload: TranscriptEventPayload, seq: number): SeqTranscriptItem | undefined {
  // `Unknown` payloads are forward-compat catch-alls; ignore them in
  // the chat body (they don't have a typed renderer).
  if (payload.item.kind === TranscriptItemKind.Unknown) {
    return undefined
  }

  // PermissionRequest rides through `acp:permission-request` for the
  // sticky stack; the transcript variant exists for typed completeness
  // but doesn't render inline.
  if (payload.item.kind === TranscriptItemKind.PermissionRequest) {
    return undefined
  }

  return {
    seq,
    turnId: payload.turnId,
    item: payload.item
  }
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

  /**
   * Pending live-event queue. `patchLatestPage` enqueues; a single
   * microtask-scheduled flush drains the whole burst into ONE
   * `setQueryData` call. Without this, a `session/load` replay
   * (hundreds of `session/update` notifications fanned out within
   * ~2ms) would land hundreds of `setQueryData` invocations on the
   * head page — each clones the entire `head.items` array (O(N))
   * and re-keys the pages array (O(pages)). Total: O(N²)
   * allocations on a hot path that drowns the renderer; under
   * extreme bursts (long session restored on a slow box) this OOM-s
   * the webview before the chat ever paints.
   *
   * Microtask scheduling collapses the burst because every
   * `acp:transcript` event handler runs synchronously off Tauri's
   * IPC bridge — the queue accumulates the whole batch on the
   * current tick, and the flush runs once after the tick yields.
   */
  let pendingPatches: TranscriptEventPayload[] = []
  let flushScheduled = false
  let stopped = false

  function flushPatches(): void {
    flushScheduled = false

    if (stopped) {
      return
    }
    const batch = pendingPatches

    pendingPatches = []

    if (batch.length === 0) {
      return
    }
    const id = instanceId.value

    if (id === undefined) {
      return
    }
    queryClient.setQueryData<PatchableInfiniteData>(['snapshot-chat', id], (old) => {
      if (!old || old.pages.length === 0) {
        // No cached pages — the live events arrived before the first
        // snapshot fetch resolved. Skip; the snapshot will land with
        // the same items via the daemon mirror.
        log.trace('snapshot.live-patch.skipped-no-cache', {
          instanceId: id,
          batchSize: batch.length
        })

        return old
      }
      const head = old.pages[0]

      if (!head) {
        return old
      }
      // Clone head.items ONCE for the whole batch.
      const nextItems = [...head.items]
      let baseSeq = (localSeq.value ?? head.latestSeq ?? 0) + 1
      let lastSeq = head.latestSeq ?? 0
      let applied = 0
      let mergedCount = 0
      let skipped = 0

      for (const payload of batch) {
        if (payload.instanceId !== id) {
          continue
        }
        const incoming = liveItemFor(payload, baseSeq)

        if (!incoming) {
          skipped += 1
          continue
        }
        const merged = mergeToolCallUpdate(nextItems, incoming)

        if (merged) {
          mergedCount += 1
        } else {
          nextItems.push(incoming)
        }
        baseSeq += 1
        lastSeq = incoming.seq
        applied += 1
      }

      if (applied === 0) {
        return old
      }

      log.trace('snapshot.live-patch.batch-applied', {
        instanceId: id,
        batchSize: batch.length,
        applied,
        merged: mergedCount,
        skipped,
        headItemCount: nextItems.length
      })

      const nextHead: ChatSnapshot = {
        items: nextItems,
        // Live patches only ever land at the head — the oldest entry
        // doesn't move, so preserve `head.oldestSeq`. If the head was
        // empty it stays unset; the next backward fetch will populate.
        oldestSeq: head.oldestSeq ?? lastSeq,
        latestSeq: lastSeq,
        hasMore: head.hasMore
      }

      return {
        ...old,
        pages: [nextHead, ...old.pages.slice(1)],
        pageParams: old.pageParams
      }
    })
  }

  function patchLatestPage(payload: TranscriptEventPayload): void {
    const id = instanceId.value

    if (id === undefined) {
      return
    }

    if (payload.instanceId !== id) {
      return
    }
    pendingPatches.push(payload)

    if (!flushScheduled) {
      flushScheduled = true
      queueMicrotask(flushPatches)
    }
  }

  function patchPermissionResolved(payload: AcpPermissionResolvedPayload): void {
    // Drop the matching row from the per-instance permissions store
    // regardless of whether the resolved id matches the focused
    // viewport — a captain answering on the desktop must clear the
    // remote's row, and vice versa, even when the captain is looking
    // at another instance at that moment. The wire payload always
    // carries `instanceId`, so addressing it directly is correct.
    usePermissions().clearById(payload.instanceId, payload.requestId)

    const id = instanceId.value

    if (id === undefined) {
      return
    }

    if (payload.instanceId !== id) {
      return
    }
    queryClient.setQueryData<MetaSnapshot>(['snapshot-meta', id], (old) => {
      if (!old) {
        return old
      }
      const pending = old.pendingPermissions

      if (!pending || pending.length === 0) {
        return old
      }
      const filtered = pending.filter((p) => p.requestId !== payload.requestId)

      if (filtered.length === pending.length) {
        return old
      }

      return { ...old, pendingPermissions: filtered }
    })
  }

  // Wire the live-event listeners. Single subscription per composable
  // instance — Overlay.vue mounts the viewport once. Cleanup runs on
  // unmount via `onUnmounted` AND through the returned `stop()` so
  // tests can tear down explicitly.
  const unlisteners: UnlistenFn[] = []

  function stop(): void {
    if (stopped) {
      return
    }
    stopped = true

    for (const u of unlisteners) {
      u()
    }
    unlisteners.length = 0
    // Drop any pending batch the next microtask was about to flush —
    // the queryClient may have been torn down by the time the flush
    // runs, and even if it hasn't, applying live patches to a
    // composable that the captain has already unmounted is wasted
    // work.
    pendingPatches = []
  }

  void (async() => {
    try {
      const t = await listen(TauriEvent.AcpTranscript, (e) => {
        patchLatestPage(e.payload)
      })

      // Race guard: stop() may have been called while the first
      // `listen()` was awaiting. Drop the registration immediately
      // and skip the second so a unit-test teardown that races the
      // IIFE doesn't leak a stale listener pointing at the old
      // queryClient.
      if (stopped) {
        t()

        return
      }
      unlisteners.push(t)
      const p = await listen(TauriEvent.AcpPermissionResolved, (e) => {
        patchPermissionResolved(e.payload)
      })

      if (stopped) {
        p()

        return
      }
      unlisteners.push(p)
    } catch {
      // Listener registration errors surface in the dev console; the
      // viewport stays functional via the snapshot path.
    }
  })()

  onUnmounted(stop)

  // Page-trim policy. Idempotent: drop pages older than the
  // newest `MAX_PAGES_KEPT` whenever the caller signals "the
  // captain is in the live area, dropping older pages won't yank
  // anything they're reading." The body view calls this from its
  // scroll handler when the captain is within ~one viewport of
  // bottom — wider than `useStickToBottom`'s 64px threshold (which
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

  // Defensive watcher — when the instanceId flips, reset the local
  // seq cursor. The infinite-query wakes its own cache key off the
  // ref so this is mostly belt-and-braces; explicit teardown of any
  // leaked listeners would land here too if the composable grew
  // multi-instance subscriptions.
  watch(instanceId, () => {
    // No-op today — `localSeq` is computed off the cache, which
    // re-keys with `instanceId.value`. Future per-instance event
    // filters would re-bind here.
  })

  return {
    items,
    latestSeq: localSeq,
    isInitialLoading,
    isFetchingNextPage,
    hasNextPage,
    fetchNextPage,
    evictExtraPages,
    stop
  }
}
