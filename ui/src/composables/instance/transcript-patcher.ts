/**
 * Module-level singleton that patches the per-instance chat cache from
 * live `acp:transcript` + `acp:permission-resolved` events.
 *
 * **Why a singleton, not a composable.** The earlier
 * `useChatViewport`-scoped listener IIFE only wired itself when the
 * `ChatViewport` Vue component mounted, which on the remote bridge
 * lands AFTER:
 *   1. WS auth → daemon auto-subscribes to events
 *   2. `applyBootSnapshot` → fires the boot-time snapshot prefetches
 *   3. `main.ts` returns → `app.mount('#app')` → `App.vue` renders
 *   4. `Overlay.vue` mounts → `onMounted` runs → `ChatViewport` renders
 *   5. `useChatViewport()` runs → IIFE awaits `listen()`
 *
 * Between steps 1 and 5, any `acp:transcript` event for an in-flight
 * turn lands at the WS dispatcher with NO listeners registered for
 * `acp:transcript`. `remote-bridge.ts::onMessage` checks
 * `eventListeners.get(name)` and silently returns when the set is
 * empty — every event in that window is dropped on the floor. The
 * daemon mirror still has them, the snapshot RPC will return them,
 * but the cache that `<Viewport>` reads from never sees them as live
 * patches.
 *
 * **The actual visible symptom on remote**: a captain reconnecting
 * while a turn is streaming sees the historical content from the
 * snapshot eventually, but the live stream looks frozen — events
 * fired while the SPA was bootstrapping never landed. New events
 * fired after the SPA finishes booting work as expected, hence the
 * captain's report "it just handles the incoming data as it goes
 * along".
 *
 * Wiring this from `main.ts` before `applyBootSnapshot` runs means
 * `eventListeners.get('acp:transcript')` is populated before the
 * very first `acp:transcript` push hits the WS dispatcher.
 *
 * **Cold-cache handling is conservative.** Most events emitted before
 * the snapshot RPC is processed end up in the snapshot (the daemon WS
 * bridge uses `tokio::select! { biased; … }` to push RPC responses
 * before broadcast events on the same connection). We still render the
 * first user-authored row / change advertisement when it arrives before
 * any snapshot cache exists so the captain sees immediate feedback,
 * but mark that cache partial so focus/switch hydration replaces it
 * with the daemon-retained transcript instead of treating the live tail
 * as complete forever.
 *
 * **Per-instance cache keying** stays as it was —
 * `['snapshot-chat', instanceId]` and `['snapshot-meta', instanceId]`.
 * The singleton just owns the listener; the dispatch is identical.
 */

import { type QueryClient } from '@tanstack/vue-query'

import { partialChatData, snapshotChatKey, type ChatInfiniteData } from './chat-cache'
import { nextSeq } from './sequence'
import { usePermissions } from './use-permissions'
import {
  invoke,
  listen,
  TauriCommand,
  TauriEvent,
  TranscriptItemKind,
  type AcpPermissionResolvedPayload,
  type ChatSnapshot,
  type MetaSnapshot,
  type SeqTranscriptItem,
  type TranscriptEventPayload,
  type UnlistenFn
} from '@ipc'
import { log } from '@lib'

/// Per-instance pending-patches queue. Events arrive faster than Vue's
/// reactive flush; collecting them in one microtask + draining as a
/// single `setQueryData` keeps O(N²) clone churn off the hot path.
const pendingByInstance = new Map<string, TranscriptEventPayload[]>()
let flushScheduled = false
let started = false
let unlisteners: UnlistenFn[] = []
/// Per-instance highest `seq` applied to the local chat cache. The remote-bridge
/// reads this on reconnect and asks the daemon for everything strictly
/// newer via `instance_snapshot_chat { after }`. Full snapshots seed it
/// from `latestSeq`; live events update it only after the queued patch
/// actually lands in the cache. Older daemons leave `seq` undefined —
/// the counter stays put and the reconnect path falls back to a
/// head-anchored fetch.
const lastSeenSeqByInstance = new Map<string, number>()

export function recordLastSeenSeq(instanceId: string, seq: number | undefined): void {
  if (typeof seq !== 'number' || !Number.isFinite(seq)) {
    return
  }
  const prior = lastSeenSeqByInstance.get(instanceId) ?? 0

  if (seq > prior) {
    lastSeenSeqByInstance.set(instanceId, seq)
  }
}

export function getLastSeenSeq(instanceId: string): number | undefined {
  return lastSeenSeqByInstance.get(instanceId)
}

export function listSeenInstanceIds(): string[] {
  return [...lastSeenSeqByInstance.keys()]
}

function liveItemFor(payload: TranscriptEventPayload, fallbackSeq: number): SeqTranscriptItem | undefined {
  if (payload.item.kind === TranscriptItemKind.Unknown) {
    return undefined
  }

  if (payload.item.kind === TranscriptItemKind.PermissionRequest) {
    return undefined
  }

  // Prefer the daemon's wire seq when present so the cached entry's
  // `seq` matches what `instance_snapshot_chat` would return for the
  // same item. Lets the delta-replay path dedup overlapping items by
  // seq instead of having to reconcile two parallel numbering
  // systems (client-synthesized vs daemon-stamped). The `fallbackSeq`
  // covers older daemons that don't ship `seq` on the wire — the
  // synthesized number is monotonic per batch flush so the cache
  // stays internally consistent.
  return {
    seq: payload.seq ?? fallbackSeq,
    turnId: payload.turnId,
    messageId: payload.messageId,
    item: payload.item
  }
}

function isUserAuthoredItem(payload: TranscriptEventPayload): boolean {
  return payload.item.kind === TranscriptItemKind.UserPrompt || payload.item.kind === TranscriptItemKind.UserText
}

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

      if (next.content && next.content.length > 0) {
        merged.content = next.content
      }
      merged.formatted = next.formatted
      merged.startedAtMs = next.startedAtMs

      if (next.completedAtMs !== undefined) {
        merged.completedAtMs = next.completedAtMs
      }
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

function flushPatchesFor(queryClient: QueryClient, instanceId: string): void {
  const batch = pendingByInstance.get(instanceId)

  if (!batch || batch.length === 0) {
    return
  }

  let refetchSeededColdCache = false
  let appliedLatestSeq: number | undefined

  queryClient.setQueryData<ChatInfiniteData>(snapshotChatKey(instanceId), (old) => {
    if (!old || old.pages.length === 0) {
      const seedItems: SeqTranscriptItem[] = []
      let latestSeq = 0
      const containsUserAuthoredItem = batch.some((payload) => payload.instanceId === instanceId && isUserAuthoredItem(payload))
      const containsChangeAdvertisement = batch.some((payload) => payload.instanceId === instanceId && payload.item.kind === TranscriptItemKind.ChangeAdvertisement)

      if (containsUserAuthoredItem || containsChangeAdvertisement) {
        for (const payload of batch) {
          if (payload.instanceId !== instanceId) {
            continue
          }
          const incoming = liveItemFor(payload, latestSeq + 1)

          if (!incoming) {
            continue
          }
          seedItems.push(incoming)
          latestSeq = Math.max(latestSeq, incoming.seq)
        }
      }

      if (seedItems.length > 0) {
        pendingByInstance.delete(instanceId)
        // Cold live seeds are only a landing pad so the captain sees
        // events that arrived before a full snapshot baseline. Mark
        // them partial so focus/switch hydration replaces them with
        // the daemon-retained transcript instead of treating a tail
        // as complete forever.
        refetchSeededColdCache = true
        appliedLatestSeq = latestSeq

        return partialChatData({
          items: seedItems,
          oldestSeq: seedItems[0]?.seq ?? latestSeq,
          latestSeq,
          hasMore: false
        })
      }
      // No cached pages yet — the snapshot RPC hasn't landed. Drop the
      // batch: per the wire-ordering invariant (`biased;` select in
      // `remote/ws.rs`), events emitted before the snapshot response is
      // sent are also in the snapshot when it lands. Holding them here
      // and re-applying after would double-paint. Clearing keeps the
      // cache consistent with the snapshot's view.
      log.trace('transcript-patcher.skipped-no-cache', {
        instanceId,
        batchSize: batch.length
      })
      pendingByInstance.delete(instanceId)

      return old
    }
    const head = old.pages[0]

    if (!head) {
      pendingByInstance.delete(instanceId)

      return old
    }
    const nextItems = [...head.items]
    let baseSeq = (head.latestSeq ?? 0) + 1
    let lastSeq = head.latestSeq ?? 0
    let applied = 0

    for (const payload of batch) {
      if (payload.instanceId !== instanceId) {
        continue
      }
      const incoming = liveItemFor(payload, baseSeq)

      if (!incoming) {
        continue
      }

      // Wire-seq dedup. If the same seq already landed via the
      // delta-replay path (rare race on remote reconnect), drop the
      // duplicate live event rather than appending a phantom copy.
      // Only meaningful when the wire ships seq — older daemons with
      // synthesized seqs never hit this branch because the
      // synthesized counter is monotonic per flush.
      if (payload.seq !== undefined && payload.seq <= lastSeq) {
        continue
      }

      if (!mergeToolCallUpdate(nextItems, incoming)) {
        nextItems.push(incoming)
      }
      baseSeq = incoming.seq + 1
      lastSeq = incoming.seq
      applied += 1
    }
    pendingByInstance.delete(instanceId)

    if (applied === 0) {
      return old
    }

    const nextHead: ChatSnapshot = {
      items: nextItems,
      oldestSeq: head.oldestSeq ?? lastSeq,
      latestSeq: lastSeq,
      hasMore: head.hasMore
    }

    appliedLatestSeq = lastSeq

    return {
      ...old,
      pages: [nextHead, ...old.pages.slice(1)],
      pageParams: old.pageParams
    }
  })

  recordLastSeenSeq(instanceId, appliedLatestSeq)

  if (refetchSeededColdCache) {
    void queryClient.invalidateQueries({ queryKey: snapshotChatKey(instanceId) })
  }
}

function scheduleFlush(queryClient: QueryClient): void {
  if (flushScheduled) {
    return
  }
  flushScheduled = true
  queueMicrotask(() => {
    flushScheduled = false
    const ids = [...pendingByInstance.keys()]

    for (const id of ids) {
      flushPatchesFor(queryClient, id)
    }
  })
}

function patchLatestPage(queryClient: QueryClient, payload: TranscriptEventPayload): void {
  // Touch the local seq counter so the singleton stays roughly in sync
  // with the per-instance composable's monotonic counter. Used for
  // diagnostic traces — independent of the daemon-minted `seq` on the
  // payload (the local counter exists for backward compat with older
  // daemons that don't ship `seq`).
  nextSeq(payload.instanceId)
  const list = pendingByInstance.get(payload.instanceId) ?? []

  list.push(payload)
  pendingByInstance.set(payload.instanceId, list)
  scheduleFlush(queryClient)
}

function patchPermissionResolved(queryClient: QueryClient, payload: AcpPermissionResolvedPayload): void {
  usePermissions().clearById(payload.instanceId, payload.requestId)
  queryClient.setQueryData<MetaSnapshot>(['snapshot-meta', payload.instanceId], (old) => {
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

/**
 * Wire the singleton listeners. Idempotent — second call returns the
 * existing teardown fn. Call from `main.ts` BEFORE the first boot RPC
 * fires so listener registration races the daemon's auto-subscribe at
 * WS handshake (events get routed to a populated map instead of an
 * empty one).
 */
export async function startTranscriptPatcher(queryClient: QueryClient): Promise<() => void> {
  if (started) {
    return stopTranscriptPatcher
  }
  started = true

  try {
    unlisteners.push(
      await listen(TauriEvent.AcpTranscript, (e) => {
        patchLatestPage(queryClient, e.payload)
      }),
      await listen(TauriEvent.AcpPermissionResolved, (e) => {
        patchPermissionResolved(queryClient, e.payload)
      })
    )
  } catch(err) {
    log.warn('transcript-patcher: listener registration failed', undefined, err)
    started = false
  }

  return stopTranscriptPatcher
}

export function stopTranscriptPatcher(): void {
  for (const u of unlisteners) {
    u()
  }
  unlisteners = []
  pendingByInstance.clear()
  lastSeenSeqByInstance.clear()
  flushScheduled = false
  started = false
}

/**
 * Test-only reset. Drops every listener + clears the pending queue.
 */
export function __resetTranscriptPatcherForTests(): void {
  stopTranscriptPatcher()
}

/// Max items pulled per delta-replay RPC. Replay loops until the
/// daemon reports exhaustion, yielding between pages so a reconnect or
/// instance switch that has thousands of missed chunks does not pin the
/// UI thread in one long cache-merge burst.
const DELTA_REPLAY_BATCH_SIZE = 25
const DELTA_REPLAY_YIELD_MS = 0

function yieldReplayTurn(): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, DELTA_REPLAY_YIELD_MS))
}

/// Outcome of [`applyChatDeltaPage`]. `kind = 'cache-cold'` means the
/// `['snapshot-chat', instanceId]` query has no cached pages yet
/// (boot hydration hadn't landed when the resync fired) — the delta
/// items were NOT applied and would silently disappear if the caller
/// returned success. `resyncFromRemote` reads the discriminant and
/// degrades to the page-reload fallback in that case so the captain
/// doesn't end up staring at a chat surface that's missing turns.
type DeltaApplyOutcome = { kind: 'cache-cold'; received: number } | { kind: 'applied'; applied: number }

/// Apply a daemon-served delta page (items strictly newer than the
/// caller's last-seen seq) onto the head of the cached chat infinite
/// query. Items are sequenced by the daemon — we trust their `seq`
/// values and merge ToolCallUpdate entries onto in-cache ToolCall
/// rows the same way the live patcher does, so a delta-replay that
/// catches a mid-streaming tool call collapses correctly instead of
/// stacking a phantom row.
function applyChatDeltaPage(queryClient: QueryClient, instanceId: string, items: SeqTranscriptItem[]): DeltaApplyOutcome {
  if (items.length === 0) {
    return { kind: 'applied', applied: 0 }
  }

  let coldCache = false
  let applied = 0

  queryClient.setQueryData<ChatInfiniteData>(snapshotChatKey(instanceId), (old) => {
    if (!old || old.pages.length === 0) {
      coldCache = true

      return old
    }
    const head = old.pages[0]

    if (!head) {
      coldCache = true

      return old
    }
    const nextItems = [...head.items]
    let lastSeq = head.latestSeq ?? 0

    for (const incoming of items) {
      // Dedup against the head's existing max seq. Covers the race
      // where a live event for the same seq landed between the
      // resync RPC being sent and its response arriving — without
      // this guard the captain would see a chunk rendered twice
      // (live + replay). Because the live patcher now stamps the
      // wire seq onto cached items, `lastSeq` is comparable across
      // both paths.
      if (incoming.seq <= lastSeq) {
        recordLastSeenSeq(instanceId, incoming.seq)
        continue
      }
      const merged = mergeToolCallUpdate(nextItems, incoming)

      if (!merged) {
        nextItems.push(incoming)
      }
      lastSeq = incoming.seq
      recordLastSeenSeq(instanceId, incoming.seq)
      applied += 1
    }

    const nextHead: ChatSnapshot = {
      items: nextItems,
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

  if (coldCache) {
    return { kind: 'cache-cold', received: items.length }
  }

  return { kind: 'applied', applied }
}

/// Per-instance outcome the orchestrator aggregates. `cache-cold`
/// means the delta page arrived but no chat query was hydrated yet
/// — those items can't be patched in-place without producing a
/// half-rendered view, so the orchestrator degrades to the page
/// reload that the bridge falls back to.
export type ReplayOutcome = 'applied' | 'cache-cold' | 'failed'

export async function replayAvailableForInstance(queryClient: QueryClient, instanceId: string): Promise<ReplayOutcome> {
  let after = lastSeenSeqByInstance.get(instanceId)

  if (after === undefined) {
    return 'applied'
  }

  try {
    for (;;) {
      const snap = (await invoke(TauriCommand.InstanceSnapshotChat, {
        instanceId,
        after,
        limit: DELTA_REPLAY_BATCH_SIZE
      })) as ChatSnapshot

      if (snap.items.length === 0) {
        log.trace('snapshot.delta-replay.empty', {
          instanceId,
          after,
          hasMore: snap.hasMore
        })

        return snap.hasMore ? 'failed' : 'applied'
      }
      const outcome = applyChatDeltaPage(queryClient, instanceId, snap.items)

      log.trace('snapshot.delta-replay.applied', {
        instanceId,
        after,
        received: snap.items.length,
        outcome,
        hasMore: snap.hasMore,
        latestSeq: snap.latestSeq
      })

      if (outcome.kind === 'cache-cold' && outcome.received > 0) {
        // We had a cursor (live event recorded a seq) but no cached
        // query pages to merge into — most often because the captain
        // hit the page before any chat component subscribed to the
        // infinite query. Surface this so the orchestrator can reload
        // rather than silently drop the items.
        return 'cache-cold'
      }

      if (!snap.hasMore) {
        return 'applied'
      }
      const nextAfter = getLastSeenSeq(instanceId) ?? snap.latestSeq

      if (nextAfter === undefined || nextAfter <= after) {
        log.warn('transcript-patcher: delta-replay cursor did not advance', {
          instanceId,
          after,
          nextAfter,
          pageSize: DELTA_REPLAY_BATCH_SIZE
        })

        return 'failed'
      }
      after = nextAfter
      await yieldReplayTurn()
    }
  } catch(err) {
    log.warn('transcript-patcher: delta-replay failed', { instanceId, after }, err)

    return 'failed'
  }
}

/**
 * Top-level resync hook for the remote bridge. Called on silent
 * reauth after a dropped WS. For every instance we've ever seen on
 * this page, pulls every available newer delta page
 * (`instance_snapshot_chat { after }`) and patches the head. Meta /
 * terminals / queue / instances list
 * stay current via vue-query invalidations — UI subscribers refetch
 * automatically.
 *
 * Returns `true` when the resync completed without falling back to
 * the page reload. `false` signals the caller (`remote-bridge.ts`)
 * to reload the page as a coarse-but-correct safety net. Degrades
 * to reload when ANY instance hit either a transport failure or the
 * "cache-cold but cursor populated" race (delta items arrived but
 * the chat query had no pages to merge into — silently dropping them
 * would leave the captain with a missing turn).
 */
export async function resyncFromRemote(queryClient: QueryClient): Promise<boolean> {
  const instanceIds = [...lastSeenSeqByInstance.keys()]

  if (instanceIds.length === 0) {
    // No prior state to delta-replay against — the page must be in
    // an unusual state (rare reconnect before any event ever
    // landed). Fall back to reload.
    return false
  }

  const outcomes: ReplayOutcome[] = []

  for (const id of instanceIds) {
    outcomes.push(await replayAvailableForInstance(queryClient, id))
    await yieldReplayTurn()
  }

  if (outcomes.some((o) => o !== 'applied')) {
    log.warn('transcript-patcher: resync degraded to reload', {
      instanceIds,
      outcomes
    })

    return false
  }

  await Promise.all([
    queryClient.invalidateQueries({ queryKey: ['snapshot-meta'] }),
    queryClient.invalidateQueries({ queryKey: ['snapshot-terminals'] }),
    queryClient.invalidateQueries({ queryKey: ['snapshot-queue'] }),
    queryClient.invalidateQueries({ queryKey: ['instances'] })
  ])

  return true
}
