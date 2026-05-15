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

import { pushCurrentModeUpdate, setInstanceAgent, setInstanceName, setInstanceProfile } from './use-session-info'
import { type InstanceId, useActiveInstance } from '../chrome/use-active-instance'
import { invoke, listen, TauriCommand, TauriEvent, type ChatSnapshot, type UnlistenFn } from '@ipc'
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
 *
 * `limit` defaults to a viewport-derived window size (proxied off
 * `window.innerHeight` since the actual chat scroller isn't mounted
 * yet at boot time). On a phone that's ~8 turns; on a desktop
 * monitor ~30 — far less than the old hard-coded 50, so the captain
 * doesn't wait on bytes they can't see.
 */
export function prefetchInstanceChatFirstPage(client: QueryClient, instanceId: InstanceId, limit?: number): Promise<void> {
  const resolvedLimit = limit ?? bootPageSize()

  return client.prefetchInfiniteQuery({
    queryKey: ['snapshot-chat', instanceId],
    initialPageParam: undefined as number | undefined,
    queryFn: () =>
      invoke(TauriCommand.InstanceSnapshotChat, {
        instanceId,
        before: undefined,
        limit: resolvedLimit
      }),
    getNextPageParam: (lastPage: ChatSnapshot) => (lastPage?.hasMore ? lastPage.oldestSeq : undefined)
  }) as unknown as Promise<void>
}

/**
 * Boot-time page size. Hard 100 — matches the nvim plugin's
 * `snapshot_limit` default and produces an immediately-readable
 * conversation on every form factor.
 *
 * The earlier viewport-derived heuristic (~96px / row, clamped at 20)
 * fetched ≤20 items on a typical 1080p monitor → captains on a long
 * session saw only the last 20 turns and the rest of the history was
 * invisible until they manually scrolled up far enough to trigger
 * `fetchNextPage`. The Vue UIs were the only frontends shipping a
 * truncated view; nvim's full-history paint felt strictly more
 * correct.
 *
 * Trade-off: a fresh boot now pulls more bytes than fit on screen.
 * That's the right call — `fetchNextPage` only loads further back
 * from the OLDEST seq, so paying for 100 up-front is the difference
 * between "captain sees their conversation" and "captain wonders
 * where their conversation went". The chat-viewport's later
 * `measuredPageSize` keeps subsequent backward fetches viewport-sized
 * so we don't multiply the cost.
 */
const BOOT_PAGE_SIZE = 100

function bootPageSize(): number {
  return BOOT_PAGE_SIZE
}

/**
 * Brim-sync: pull `instances/list` and prime the meta cache for every
 * instance. `instances/list` now ships `focusedId` alongside the list
 * — when `useActiveInstance` is empty (a remote that just authenticated
 * mid-session, before any `acp:instances-focused` event has fired),
 * the daemon's focused id seeds the active-instance pointer via
 * `applyFocus('manual')`. The first chat page for that focused id is
 * prefetched so the chat surface paints from the daemon's snapshot
 * without waiting for the next live event.
 *
 * The caller-supplied `localFocusedId` (the captain's currently-active
 * instance, if any) overrides the daemon-reported focus — desktop
 * captains who clicked into a non-focused instance have their local
 * choice preserved across re-syncs.
 *
 * All fetches fire in parallel; the returned promise resolves once
 * every prefetch settles (success OR error — `prefetchQuery` swallows
 * rejections by design so a partial sync doesn't take the brim down).
 */
export async function brimSync(client: QueryClient, localFocusedId?: InstanceId): Promise<void> {
  log.trace('snapshot.brim-sync.start', { localFocusedId })
  let instanceIds: InstanceId[]
  let daemonFocusedId: InstanceId | undefined

  try {
    const r = await invoke(TauriCommand.InstancesList)

    instanceIds = r.instances.map((entry) => entry.instanceId)
    daemonFocusedId = r.focusedId

    // Seed `useSessionInfo` from each instance entry. The
    // `instances/list` payload carries `agentId` / `profileId` /
    // `mode` / `name` per instance — without this push, a remote
    // captain's header pills (agent / profile / mode / name) stay
    // empty until the next live event for the instance fires
    // (often only after a turn). The `useSnapshotHydration` hook
    // covers cwd / model / configOptions / mcpsCount via the meta
    // query; this covers the fields the meta snapshot doesn't.
    // `!= null` covers null AND undefined — the wire shape is supposed
    // to omit unset fields, but a buggy daemon build (or a spec drift)
    // could still ship `null`. `entry.name !== undefined && null.length`
    // would throw; the loose check defends against that regression
    // shape silently passing through to a runtime crash.
    for (const entry of r.instances) {
      if (entry.agentId) {
        setInstanceAgent(entry.instanceId, entry.agentId)
      }

      if (entry.profileId != null) {
        setInstanceProfile(entry.instanceId, entry.profileId)
      }

      if (entry.mode != null) {
        pushCurrentModeUpdate(entry.instanceId, { currentModeId: entry.mode })
      }

      if (entry.name != null && entry.name.length > 0) {
        setInstanceName(entry.instanceId, entry.name)
      }
    }
  } catch(err) {
    log.warn('brim-sync: instances_list failed', { err: String(err) })

    return
  }

  // Seed `useActiveInstance` from the daemon's focus pointer when the
  // caller has no local choice — covers the remote-authenticated-mid-
  // session case. Captains who passed `localFocusedId` keep their
  // choice; the daemon focus only fills the empty slot.
  const effectiveFocus = localFocusedId ?? daemonFocusedId

  if (effectiveFocus) {
    useActiveInstance().setIfUnset(effectiveFocus)
  }

  log.trace('snapshot.brim-sync.resolved', {
    instanceCount: instanceIds.length,
    daemonFocusedId,
    localFocusedId,
    effectiveFocus
  })

  const tasks: Promise<void>[] = []

  for (const id of instanceIds) {
    tasks.push(prefetchInstanceMeta(client, id))
  }

  if (effectiveFocus) {
    tasks.push(prefetchInstanceChatFirstPage(client, effectiveFocus))
  }
  await Promise.all(tasks)
  log.trace('snapshot.brim-sync.done', {
    instanceCount: instanceIds.length,
    chatPrimed: effectiveFocus !== undefined
  })
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
        log.trace('snapshot.focus-prefetch.focused', { instanceId: next })
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
        log.trace('snapshot.focus-prefetch.changed', {
          instanceIds: e.payload.instanceIds,
          focusedId: e.payload.focusedId
        })

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
