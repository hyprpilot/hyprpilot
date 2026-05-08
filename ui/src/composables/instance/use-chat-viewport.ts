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
import { computed, onUnmounted, watch, type ComputedRef } from 'vue'

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

/** Cache pages retained when stuck-at-bottom. */
export const MAX_PAGES_KEPT = 3

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
   * Notify the viewport that the captain is stuck-at-bottom. Triggers
   * page-trim when the cache exceeds `MAX_PAGES_KEPT`. Called by the
   * body component from a `watch` on the `useStickToBottom` signal.
   */
  onStuckChange: (stuck: boolean) => void
  /** Unsubscribe the live-event listeners — wired automatically on unmount. */
  stop: () => void
}

interface PatchableInfiniteData extends InfiniteData<ChatSnapshot, number | undefined> {
  pages: ChatSnapshot[]
  pageParams: (number | undefined)[]
}

/**
 * Merge a tool-call-update item onto an existing tool-call (or
 * tool-call-update) entry on the latest page. Returns `true` when a
 * merge happened — the caller then skips the append branch.
 *
 * Wire shape: ACP `tool_call_update` carries the same `id` as the
 * original `tool_call`; the daemon mirror holds the merged record,
 * but live patches arrive as raw deltas. Field-by-field: only
 * defined values from the update overwrite the prior record so an
 * agent can stream `formatted` updates without re-shipping
 * `rawInput` / `startedAtMs`.
 */
function mergeToolCallUpdate(items: SeqTranscriptItem[], incoming: SeqTranscriptItem): boolean {
  const next = incoming.item

  if (next.kind !== TranscriptItemKind.ToolCallUpdate && next.kind !== TranscriptItemKind.ToolCall) {
    return false
  }

  // Only `ToolCallUpdate` triggers a merge — initial `ToolCall`
  // entries always append. The daemon emits the initial `ToolCall`
  // exactly once per call.
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
      // Field-by-field overwrite: only defined values from the update
      // replace the prior values. Mirrors the daemon's
      // `tool_call_cache.merge` semantics so the UI's view matches the
      // mirror's view.
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
      // Content arrays replace wholesale — the daemon's
      // `ToolCallContent` is treated as a snapshot per delta, not
      // appended.
      merged.content = next.content
      merged.formatted = next.formatted
      merged.startedAtMs = next.startedAtMs

      if (next.completedAtMs !== undefined) {
        merged.completedAtMs = next.completedAtMs
      }
      items[i] = { seq: existing.seq, item: merged }

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
    seq, turnId: payload.turnId, item: payload.item
  }
}

export function useChatViewport(instanceId: ComputedRef<InstanceId | undefined>): UseChatViewportApi {
  const query: UseInstanceChatInfiniteQueryReturn = useInstanceChatInfiniteQuery(instanceId)
  const queryClient = useQueryClient()

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

  function patchLatestPage(payload: TranscriptEventPayload): void {
    const id = instanceId.value

    if (id === undefined) {
      return
    }

    if (payload.instanceId !== id) {
      return
    }
    queryClient.setQueryData<PatchableInfiniteData>(['snapshot-chat', id], (old) => {
      if (!old || old.pages.length === 0) {
        // No cached pages — the live event arrived before the first
        // snapshot fetch resolved. Skip; the snapshot will land with
        // the same item via the daemon mirror.
        log.trace('snapshot.live-patch.skipped-no-cache', {
          instanceId: id,
          itemKind: payload.item.kind
        })

        return old
      }
      // The newest page is page 0 (daemon contract: "anchor at the
      // latest seq").
      const head = old.pages[0]

      if (!head) {
        return old
      }
      const baseSeq = (localSeq.value ?? head.latestSeq ?? 0) + 1
      const incoming = liveItemFor(payload, baseSeq)

      if (!incoming) {
        log.trace('snapshot.live-patch.skipped-non-rendered', {
          instanceId: id,
          itemKind: payload.item.kind
        })

        return old
      }
      const nextItems = [...head.items]
      const merged = mergeToolCallUpdate(nextItems, incoming)

      if (!merged) {
        nextItems.push(incoming)
      }
      const nextHead: ChatSnapshot = {
        items: nextItems,
        oldestSeq: head.oldestSeq ?? incoming.seq,
        latestSeq: incoming.seq,
        hasMore: head.hasMore
      }
      const nextPages = [nextHead, ...old.pages.slice(1)]

      log.trace('snapshot.live-patch.applied', {
        instanceId: id,
        itemKind: payload.item.kind,
        merged,
        seq: incoming.seq,
        headItemCount: nextItems.length
      })

      return {
        ...old,
        pages: nextPages,
        pageParams: old.pageParams
      }
    })
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
  let stopped = false

  function stop(): void {
    if (stopped) {
      return
    }
    stopped = true

    for (const u of unlisteners) {
      u()
    }
    unlisteners.length = 0
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

  // Page-trim policy. Fires only when transitioning into stuck-at-
  // bottom AND the cache exceeds the kept-page budget. We avoid
  // trimming on every scroll tick — `setQueryData` is observed by
  // every subscriber and would re-render the body on each call.
  function onStuckChange(stuck: boolean): void {
    if (!stuck) {
      return
    }
    const id = instanceId.value

    if (id === undefined) {
      return
    }
    queryClient.setQueryData<PatchableInfiniteData>(['snapshot-chat', id], (old) => {
      if (!old || old.pages.length <= MAX_PAGES_KEPT) {
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
    onStuckChange,
    stop
  }
}
