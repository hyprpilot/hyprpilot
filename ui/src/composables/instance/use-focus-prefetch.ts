/**
 * Brim-sync + per-focus snapshot prefetch (Phase C2).
 *
 * Two responsibilities:
 *
 * 1. **Brim-sync on first authenticate.** When the SPA goes online
 *    (desktop: at mount; remote: when the WS upgrades to
 *    `authenticated`), pull `instances/list` and prefetch
 *    `['snapshot-meta', id]` for every known instance — primes the
 *    header chrome / pickers / pending-permissions row for instant
 *    focus-switch. The currently-focused instance also gets its
 *    first chat page primed.
 *
 * 2. **Per-focus prefetch on `acp:instances-focused` /
 *    `acp:instances-changed`.** Every time the daemon's focus pointer
 *    moves to a new id (or a new instance enters the registry), the
 *    listener prefetches that id's meta + first chat page so the
 *    transition into the new viewport is instant. `prefetchQuery` is
 *    idempotent — a fresh cache hit short-circuits, so the listener
 *    can fire every time without burning RPCs.
 *
 * Snapshot fetches run in parallel; the UI is fine to render before
 * any of them settle (the chat viewport's `useInfiniteQuery` already
 * runs its own first-page fetch off the focused id; this is layered
 * pre-warming, not the data path).
 */

import { useQueryClient, type QueryClient } from '@tanstack/vue-query'

import { DEFAULT_CHAT_LIMIT } from './use-instance-chat-infinite-query'
import { type InstanceId } from '../chrome/use-active-instance'
import {
  invoke,
  listen,
  TauriCommand,
  TauriEvent,
  type ChatSnapshot,
  type UnlistenFn
} from '@ipc'
import { log } from '@lib'

/**
 * Prefetch the meta snapshot for one instance. Idempotent — TanStack
 * dedupes on the queryKey, so concurrent callers share one in-flight
 * request and a fresh cache entry is reused.
 */
export function prefetchInstanceMeta(client: QueryClient, instanceId: InstanceId): Promise<void> {
  return client.prefetchQuery({
    queryKey: ['snapshot-meta', instanceId],
    queryFn: () => invoke(TauriCommand.InstanceSnapshotMeta, { instanceId })
  }) as unknown as Promise<void>
}

/**
 * Prefetch the first page of the chat snapshot for one instance.
 * Mirrors `useInstanceChatInfiniteQuery`'s key + queryFn so the
 * eventual `useInfiniteQuery` mount picks up the cached first page
 * instead of refetching.
 *
 * `prefetchInfiniteQuery` is the right primitive — `prefetchQuery`
 * shapes the cache as a single-page result, which `useInfiniteQuery`
 * would then discard.
 */
export function prefetchInstanceChatFirstPage(client: QueryClient, instanceId: InstanceId): Promise<void> {
  return client.prefetchInfiniteQuery({
    queryKey: ['snapshot-chat', instanceId],
    initialPageParam: undefined as number | undefined,
    queryFn: () =>
      invoke(TauriCommand.InstanceSnapshotChat, {
        instanceId,
        before: undefined,
        limit: DEFAULT_CHAT_LIMIT
      }),
    getNextPageParam: (lastPage: ChatSnapshot) => (lastPage?.hasMore ? lastPage.oldestSeq : undefined)
  }) as unknown as Promise<void>
}

/**
 * Brim-sync: pull `instances/list` and prime the meta cache for every
 * instance. The optional `focusedId` also gets its first chat page
 * primed — passing `undefined` skips the chat prefetch (no instance
 * focused yet). All fetches fire in parallel and the returned promise
 * resolves only when every prefetch has settled (success OR error —
 * `prefetchQuery` swallows rejections by design so a partial sync
 * doesn't take the brim down).
 */
export async function brimSync(client: QueryClient, focusedId?: InstanceId): Promise<void> {
  let instanceIds: InstanceId[]

  try {
    const r = await invoke(TauriCommand.InstancesList)

    instanceIds = r.instances.map((entry) => entry.instanceId)
  } catch(err) {
    log.warn('brim-sync: instances_list failed', { err: String(err) })

    return
  }

  const tasks: Promise<void>[] = []

  for (const id of instanceIds) {
    tasks.push(prefetchInstanceMeta(client, id))
  }

  if (focusedId !== undefined) {
    tasks.push(prefetchInstanceChatFirstPage(client, focusedId))
  }
  await Promise.all(tasks)
}

export interface UseFocusPrefetchApi {
  /**
   * Run the brim-sync once. Captain calls this from `onMounted`
   * (desktop) or after the remote bridge fires `authenticated`
   * (browser). Idempotent — safe to call repeatedly though there's no
   * reason to.
   */
  brimSync: (focusedId?: InstanceId) => Promise<void>
  /**
   * Subscribe to `acp:instances-focused` + `acp:instances-changed`.
   * Resolves to a teardown thunk that drops every listener. Pair with
   * `onUnmounted(stop)`.
   */
  start: () => Promise<() => void>
}

/**
 * Build the prefetch surface. Pass the QueryClient explicitly so this
 * can be used outside a Vue setup context (e.g. from the remote-bridge
 * authenticate handler in `App.vue`).
 */
export function useFocusPrefetch(client?: QueryClient): UseFocusPrefetchApi {
  const queryClient = client ?? useQueryClient()

  async function start(): Promise<() => void> {
    const unlisteners: UnlistenFn[] = []

    unlisteners.push(
      await listen(TauriEvent.AcpInstancesFocused, (e) => {
        const next = e.payload.instanceId

        if (next === undefined) {
          return
        }
        void prefetchInstanceMeta(queryClient, next).catch((err: unknown) => {
          log.warn('focus-prefetch: meta failed', { instanceId: next, err: String(err) })
        })
        void prefetchInstanceChatFirstPage(queryClient, next).catch((err: unknown) => {
          log.warn('focus-prefetch: chat failed', { instanceId: next, err: String(err) })
        })
      }),
      await listen(TauriEvent.AcpInstancesChanged, (e) => {
        // New-instance prefetch — the captain may switch focus to any
        // of these momentarily. Meta only; the chat prefetch waits
        // for an actual focus event to avoid pulling pages the captain
        // never looks at. `prefetchQuery` dedupes so already-cached
        // entries short-circuit.
        for (const id of e.payload.instanceIds) {
          void prefetchInstanceMeta(queryClient, id).catch((err: unknown) => {
            log.warn('focus-prefetch: meta failed (instances-changed)', { instanceId: id, err: String(err) })
          })
        }
      })
    )

    return () => {
      for (const u of unlisteners) {
        u()
      }
      unlisteners.length = 0
    }
  }

  return {
    brimSync: (focusedId) => brimSync(queryClient, focusedId),
    start
  }
}
