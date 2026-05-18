/**
 * TanStack Query wrapper around the daemon's
 * `instance_snapshot_chat` Tauri command. Drives the chat surface's
 * windowed viewport (Phase C1) — the first page anchors at the
 * latest turn (`before = undefined`); calling `fetchNextPage`
 * passes `before = oldestSeq` of the previous page, so successive
 * pages reach further back in history. `getPreviousPageParam`
 * always returns `undefined` because forward pagination doesn't
 * exist — live `acp:transcript` events update the latest page
 * directly via `setQueryData` instead.
 *
 * Disabled when no instance is in focus. The caller wires intersection-
 * triggered `fetchNextPage` calls at the top of the scroll viewport
 * (Phase C1).
 */

import { useInfiniteQuery } from '@tanstack/vue-query'
import { computed, type ComputedRef } from 'vue'

import { type InstanceId } from '../chrome/use-active-instance'
import { invoke, TauriCommand, type ChatSnapshot } from '@ipc'

/**
 * Fallback page size when no caller supplies one. Used by the
 * `prefetchInstanceChatFirstPage` brim-sync helper that doesn't have
 * a viewport handle; in production the chat viewport always supplies
 * a viewport-derived page size via `useChatViewport`. Mirrors the
 * daemon's `DEFAULT_CHAT_LIMIT` for symmetry.
 */
export const DEFAULT_CHAT_LIMIT = 50

export interface UseInstanceChatInfiniteQueryOptions {
  /**
   * Page size — either a fixed number or a getter that re-reads on
   * every fetch. Pass a getter when the page size depends on
   * viewport height so a window resize on the next backward fetch
   * pulls the right number of turns. The query key intentionally
   * does NOT include the limit, so changing it doesn't invalidate
   * existing pages — only future fetches honour the new value.
   */
  limit?: number | (() => number)
}

export type UseInstanceChatInfiniteQueryReturn = ReturnType<
  typeof useInfiniteQuery<ChatSnapshot, Error, { pages: ChatSnapshot[]; pageParams: (number | undefined)[] }, unknown[], number | undefined>
>

export function useInstanceChatInfiniteQuery(instanceId: ComputedRef<InstanceId | undefined>, opts: UseInstanceChatInfiniteQueryOptions = {}): UseInstanceChatInfiniteQueryReturn {
  const resolveLimit = (): number => {
    if (typeof opts.limit === 'function') {
      return opts.limit()
    }

    return opts.limit ?? DEFAULT_CHAT_LIMIT
  }

  return useInfiniteQuery({
    queryKey: computed(() => ['snapshot-chat', instanceId.value]),
    enabled: computed(() => instanceId.value !== undefined),
    initialPageParam: undefined as number | undefined,
    queryFn: async({ pageParam }) => {
      const id = instanceId.value

      if (id === undefined) {
        throw new Error('useInstanceChatInfiniteQuery: instanceId is undefined')
      }

      try {
        return await invoke(TauriCommand.InstanceSnapshotChat, {
          instanceId: id,
          before: pageParam,
          limit: resolveLimit()
        })
      } catch(err) {
        // `instance_snapshot_chat` rejects with "not found in registry"
        // when the instance was minted client-side (palette "new") but
        // the actor hasn't spawned yet — actors are lazy, spawned only
        // on the first `session_submit`. Returning an empty snapshot
        // keeps the query in the SUCCESS state so:
        //   1. The idle/empty landing renders immediately.
        //   2. `transcript-patcher`'s `setQueryData` can populate
        //      pages[0] the moment live events arrive after the captain
        //      sends a first prompt — rather than hitting the
        //      "cache cold" guard and dropping frames.
        // Only the initial (pageParam=undefined) fetch needs this grace
        // path; backward-pagination fetches for a non-existent instance
        // are genuine errors.
        if (pageParam === undefined) {
          const emptySnapshot: ChatSnapshot = {
            items: [],
            hasMore: false,
            oldestSeq: undefined,
            latestSeq: undefined
          }

          return emptySnapshot
        }
        throw err
      }
    },
    // Live `acp:transcript` events are the source of truth for the
    // head page — `transcript-patcher` mutates the cache directly via
    // `setQueryData`. `staleTime: Infinity` blocks vue-query's
    // automatic refetch on focus / mount / interval, which would
    // otherwise clobber those patches with a stale daemon snapshot.
    // The captain triggers a true refetch only via instance switch
    // (which re-keys the query) or via the resync handler on remote
    // reconnect. The cache lives until the query is garbage-collected
    // per the parent `QueryClient`'s `gcTime`.
    staleTime: Infinity,
    getNextPageParam: (lastPage) => {
      if (!lastPage?.hasMore) {
        return undefined
      }

      // Defensive: an empty page with `hasMore: true` (theoretical
      // — daemon doesn't currently produce this) would have
      // `oldestSeq: undefined`. Passing `undefined` to the queryFn
      // would re-trigger the initial fetch and infinite-loop. Bail
      // out cleanly instead.
      return lastPage.oldestSeq ?? undefined
    },
    // Forward pagination doesn't exist — live events mutate the
    // latest page in place via `setQueryData`. Returning `undefined`
    // keeps `hasPreviousPage` false; callers don't need to gate on it.
    getPreviousPageParam: () => undefined
  })
}
