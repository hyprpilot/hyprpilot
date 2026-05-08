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

/** Default page size when callers don't override. Mirrors the daemon's `DEFAULT_CHAT_LIMIT`. */
export const DEFAULT_CHAT_LIMIT = 50

export interface UseInstanceChatInfiniteQueryOptions {
  /** Page size override; defaults to `DEFAULT_CHAT_LIMIT` (50). */
  limit?: number
}

export type UseInstanceChatInfiniteQueryReturn = ReturnType<
  typeof useInfiniteQuery<ChatSnapshot, Error, { pages: ChatSnapshot[]; pageParams: (number | undefined)[] }, unknown[], number | undefined>
>

export function useInstanceChatInfiniteQuery(instanceId: ComputedRef<InstanceId | undefined>, opts: UseInstanceChatInfiniteQueryOptions = {}): UseInstanceChatInfiniteQueryReturn {
  const limit = opts.limit ?? DEFAULT_CHAT_LIMIT

  return useInfiniteQuery({
    queryKey: computed(() => ['snapshot-chat', instanceId.value]),
    enabled: computed(() => instanceId.value !== undefined),
    initialPageParam: undefined as number | undefined,
    queryFn: async({ pageParam }) => {
      const id = instanceId.value

      if (id === undefined) {
        throw new Error('useInstanceChatInfiniteQuery: instanceId is undefined')
      }

      return invoke(TauriCommand.InstanceSnapshotChat, {
        instanceId: id,
        before: pageParam,
        limit
      })
    },
    getNextPageParam: (lastPage) => (lastPage?.hasMore ? lastPage.oldestSeq : undefined),
    // Forward pagination doesn't exist — live events mutate the
    // latest page in place via `setQueryData`. Returning `undefined`
    // keeps `hasPreviousPage` false; callers don't need to gate on it.
    getPreviousPageParam: () => undefined
  })
}
