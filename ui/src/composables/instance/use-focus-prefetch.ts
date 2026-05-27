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
 *    chat snapshot primed.
 *
 * 2. **Per-focus hydration on `acp:instances-focused` /
 *    `acp:instances-changed`.** Every time the daemon's focus pointer
 *    moves to a new id (or a new instance enters the registry), the
 *    listener prefetches cache-miss chats and delta-replays cache-hit
 *    chats. Chat work runs sequentially with event-loop yields between
 *    instances so a large restore / reconnect does not spike the UI.
 *
 * Snapshot fetches do not block first paint; the chat viewport's
 * `useInfiniteQuery` can still run its own full-ring fetch if the
 * captain reaches an instance before the pre-warm lands.
 */

import { useQueryClient, type QueryClient } from '@tanstack/vue-query'

import { replayAvailableForInstance, recordLastSeenSeq, type ReplayOutcome } from './transcript-patcher'
import { FULL_CHAT_LIMIT } from './use-instance-chat-infinite-query'
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
 * Prefetch the chat snapshot for one instance.
 * Mirrors `useInstanceChatInfiniteQuery`'s key + queryFn so the
 * eventual `useInfiniteQuery` mount picks up the cached full-ring
 * snapshot instead of refetching.
 *
 * `prefetchInfiniteQuery` is the right primitive — `prefetchQuery`
 * shapes the cache as a single-page result, which `useInfiniteQuery`
 * would then discard.
 *
 * `limit` defaults to the full daemon transcript ring so a
 * prefetched cache entry matches what the mounted chat viewport
 * expects. A smaller snapshot would stay resident forever because
 * the snapshot query is intentionally `staleTime: Infinity`.
 */
async function fetchInstanceChatSnapshot(instanceId: InstanceId, limit: number): Promise<ChatSnapshot> {
  const snap = (await invoke(TauriCommand.InstanceSnapshotChat, {
    instanceId,
    before: undefined,
    limit
  })) as ChatSnapshot

  recordLastSeenSeq(instanceId, snap.latestSeq)

  return snap
}

export function prefetchInstanceChat(client: QueryClient, instanceId: InstanceId, limit?: number): Promise<void> {
  const resolvedLimit = limit ?? FULL_CHAT_LIMIT

  return client.prefetchInfiniteQuery({
    queryKey: ['snapshot-chat', instanceId],
    initialPageParam: undefined as number | undefined,
    queryFn: () => fetchInstanceChatSnapshot(instanceId, resolvedLimit),
    getNextPageParam: (_lastPage: ChatSnapshot) => undefined,
    staleTime: Infinity
  }) as unknown as Promise<void>
}

async function refreshInstanceChat(client: QueryClient, instanceId: InstanceId, limit?: number): Promise<void> {
  const resolvedLimit = limit ?? FULL_CHAT_LIMIT
  const snap = await fetchInstanceChatSnapshot(instanceId, resolvedLimit)

  client.setQueryData(['snapshot-chat', instanceId], {
    pages: [snap],
    pageParams: [undefined as number | undefined]
  })
}

function yieldHydrationTurn(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, 0))
}

const pendingHydrationIds = new Set<InstanceId>()
let hydrationDrain: Promise<void> | undefined

function scheduleChatHydration(client: QueryClient, instanceIds: InstanceId[]): Promise<void> {
  for (const id of instanceIds) {
    pendingHydrationIds.add(id)
  }

  if (!hydrationDrain) {
    hydrationDrain = drainChatHydration(client).finally(() => {
      hydrationDrain = undefined
    })
  }

  return hydrationDrain
}

export function __resetFocusPrefetchForTests(): void {
  pendingHydrationIds.clear()
  hydrationDrain = undefined
}

async function drainChatHydration(client: QueryClient): Promise<void> {
  while (pendingHydrationIds.size > 0) {
    const id = pendingHydrationIds.values().next().value

    if (id === undefined) {
      return
    }
    pendingHydrationIds.delete(id)

    if (client.getQueryData(['snapshot-chat', id]) === undefined) {
      await prefetchInstanceChat(client, id)
    } else {
      const outcome: ReplayOutcome = await replayAvailableForInstance(client, id)

      if (outcome !== 'applied') {
        log.warn('focus-prefetch: chat replay failed; falling back to full snapshot', { instanceId: id, outcome })
        await refreshInstanceChat(client, id)
      }
    }
    await yieldHydrationTurn()
  }
}

/**
 * Brim-sync: pull `instances/list` and prime the meta cache for every
 * instance. `instances/list` now ships `focusedId` alongside the list
 * — when `useActiveInstance` is empty (a remote that just authenticated
 * mid-session, before any `acp:instances-focused` event has fired),
 * the daemon's focused id seeds the active-instance pointer via
 * `applyFocus('manual')`. Chat hydration then walks every listed
 * instance so switching focus later replays everything available.
 *
 * The caller-supplied `localFocusedId` (the captain's currently-active
 * instance, if any) overrides the daemon-reported focus — desktop
 * captains who clicked into a non-focused instance have their local
 * choice preserved across re-syncs.
 *
 * Meta fetches fire in parallel; chat hydration is sequential and
 * yields between instances to avoid a large restore/reconnect
 * overwhelming the browser thread.
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

  await Promise.all(instanceIds.map((id) => prefetchInstanceMeta(client, id)))
  await scheduleChatHydration(client, instanceIds)
  log.trace('snapshot.brim-sync.done', {
    instanceCount: instanceIds.length,
    chatPrimed: instanceIds.length > 0
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

        void scheduleChatHydration(queryClient, [next]).catch((err: unknown) => {
          log.warn('focus-prefetch: chat failed', { instanceId: next, err: String(err) })
        })
      }),
      await listen(TauriEvent.AcpInstancesChanged, (e) => {
        // New-instance prefetch — fires for every membership change
        // (spawn / shutdown / restart from any client). Meta + chat
        // both fetched so the captain navigating to a freshly-spawned
        // instance (nvim spawn, ctl spawn, session_load mint, or a
        // peer client's spawn) sees full history on first frame.
        // Without the chat prefetch here, replay events for a new id
        // can arrive before any cache exists, hit the patcher's
        // "no cache → drop" guard, and force a later full-ring fetch
        // to reconstruct what the live path missed.
        //
        // Chat hydration is gradual: cache misses fetch the full
        // retained ring; warm caches delta-replay anything newer than
        // their last seen seq. The module-level drain coalesces rapid
        // focus/change events and processes ids sequentially.
        log.trace('snapshot.focus-prefetch.changed', {
          instanceIds: e.payload.instanceIds,
          focusedId: e.payload.focusedId
        })

        for (const id of e.payload.instanceIds) {
          void prefetchInstanceMeta(queryClient, id).catch((err: unknown) => {
            log.warn('focus-prefetch: meta failed (instances-changed)', { instanceId: id, err: String(err) })
          })
        }

        void scheduleChatHydration(queryClient, e.payload.instanceIds).catch((err: unknown) => {
          log.warn('focus-prefetch: chat failed (instances-changed)', { err: String(err) })
        })
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
