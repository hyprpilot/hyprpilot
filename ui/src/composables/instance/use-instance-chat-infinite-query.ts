/**
 * TanStack Query wrapper around the daemon's
 * `instance_snapshot_chat` Tauri command. The Vue chat surface asks
 * for the daemon's full transcript ring in one snapshot and keeps the
 * infinite-query cache shape only because boot seeding + live patching
 * already speak that structure.
 *
 * Older-page lazy loading is intentionally disabled. Live
 * `acp:transcript` events update the latest page directly via
 * `setQueryData`, while remote reconnect uses the separate
 * `instance_snapshot_chat { after }` delta path in
 * `transcript-patcher.ts`.
 */

import { useInfiniteQuery } from '@tanstack/vue-query'
import { computed, type ComputedRef } from 'vue'

import { recordLastSeenSeq } from './transcript-patcher'
import { type InstanceId } from '../chrome/use-active-instance'
import { invoke, TauriCommand, type ChatSnapshot } from '@ipc'

/** Daemon mirror transcript-ring size. Frontend snapshots ask for the whole retained ring. */
export const FULL_CHAT_LIMIT = 5_000

export interface UseInstanceChatInfiniteQueryOptions {
  /** Fixed snapshot size. Defaults to the full daemon transcript ring. */
  limit?: number
}

export type UseInstanceChatInfiniteQueryReturn = ReturnType<
  typeof useInfiniteQuery<ChatSnapshot, Error, { pages: ChatSnapshot[]; pageParams: (number | undefined)[] }, unknown[], number | undefined>
>

export function useInstanceChatInfiniteQuery(instanceId: ComputedRef<InstanceId | undefined>, opts: UseInstanceChatInfiniteQueryOptions = {}): UseInstanceChatInfiniteQueryReturn {
  const limit = computed(() => opts.limit ?? FULL_CHAT_LIMIT)

  return useInfiniteQuery({
    queryKey: computed(() => ['snapshot-chat', instanceId.value]),
    enabled: computed(() => instanceId.value !== undefined),
    initialPageParam: undefined as number | undefined,
    queryFn: async({ pageParam }) => {
      const id = instanceId.value

      if (id === undefined) {
        throw new Error('useInstanceChatInfiniteQuery: instanceId is undefined')
      }

      const snap = await invoke(TauriCommand.InstanceSnapshotChat, {
        instanceId: id,
        before: pageParam,
        limit: limit.value
      })

      // Seed the delta-replay cursor whenever a snapshot page lands.
      // Head-page fetches (pageParam undefined) carry the freshest
      // `latestSeq` — that's the right baseline for "what the daemon
      // believes the highest seq is right now". Delta replay also
      // uses max() inside `recordLastSeenSeq`, so the cursor remains
      // monotonic if a manual refetch ever lands out of order.
      recordLastSeenSeq(id, snap.latestSeq)

      return snap
    },
    // Live `acp:transcript` events are the source of truth for the
    // head page — `transcript-patcher` mutates the cache directly via
    // `setQueryData`. `staleTime: Infinity` blocks vue-query's
    // automatic refetch on focus / mount / interval, which would
    // otherwise clobber those patches with a stale daemon snapshot.
    staleTime: Infinity,
    // The frontend no longer lazy-loads older windows. The initial
    // snapshot asks for the whole daemon ring; reconnect/newer-message
    // recovery uses the independent `after` delta path.
    getNextPageParam: () => undefined,
    getPreviousPageParam: () => undefined
  })
}
