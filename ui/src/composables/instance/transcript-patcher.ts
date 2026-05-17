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
 * **Drop semantics when no cache exists are preserved.** Events
 * emitted before the snapshot RPC is processed end up in the
 * snapshot (the daemon WS bridge uses `tokio::select! { biased; … }`
 * to push RPC responses before broadcast events on the same
 * connection, so on the wire a snapshot response always precedes any
 * event emitted between snapshot-fire and snapshot-response). Events
 * emitted AFTER snapshot-fire but before cache-populate arrive on
 * the client AFTER the snapshot response — by the time
 * `patchLatestPage` runs, the cache exists.
 *
 * **Per-instance cache keying** stays as it was —
 * `['snapshot-chat', instanceId]` and `['snapshot-meta', instanceId]`.
 * The singleton just owns the listener; the dispatch is identical.
 */

import { type InfiniteData, type QueryClient } from '@tanstack/vue-query'

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

interface PatchableInfiniteData extends InfiniteData<ChatSnapshot, number | undefined> {
  pages: ChatSnapshot[]
  pageParams: (number | undefined)[]
}

/// Per-instance pending-patches queue. Events arrive faster than Vue's
/// reactive flush; collecting them in one microtask + draining as a
/// single `setQueryData` keeps O(N²) clone churn off the hot path.
const pendingByInstance = new Map<string, TranscriptEventPayload[]>()
let flushScheduled = false
let started = false
let unlisteners: UnlistenFn[] = []
/// Per-instance highest `seq` observed on the wire. The remote-bridge
/// reads this on reconnect and asks the daemon for everything strictly
/// newer via `instance_snapshot_chat { after }`. Seeded from snapshot
/// pages (the `latestSeq` field on `ChatSnapshot`); updated on every
/// live `acp:transcript` event whose payload carries `seq`. Older
/// daemons leave `seq` undefined — the counter stays put and the
/// reconnect path falls back to a head-anchored fetch.
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

/// Find the most-recent existing item in the cache that an incoming
/// `AgentText` / `AgentThought` chunk should accumulate onto. Match
/// is by `(turnId, kind)` — one accumulator per turn per role. Walks
/// back-to-front because the captain's reading position is at the
/// foot; the youngest matching entry is the live accumulator.
function findAccumulatorIndex(items: SeqTranscriptItem[], kind: TranscriptItemKind, turnId: string | undefined): number {
  if (turnId === undefined) {
    return -1
  }

  for (let i = items.length - 1; i >= 0; i -= 1) {
    const it = items[i]

    if (!it) {
      continue
    }

    if (it.item.kind === kind && it.turnId === turnId) {
      return i
    }
  }

  return -1
}

function liveItemFor(payload: TranscriptEventPayload, seq: number): SeqTranscriptItem | undefined {
  if (payload.item.kind === TranscriptItemKind.Unknown) {
    return undefined
  }

  if (payload.item.kind === TranscriptItemKind.PermissionRequest) {
    return undefined
  }

  return {
    seq,
    turnId: payload.turnId,
    item: payload.item
  }
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
        merged.content = [...(ex.content ?? []), ...next.content]
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

  queryClient.setQueryData<PatchableInfiniteData>(['snapshot-chat', instanceId], (old) => {
    if (!old || old.pages.length === 0) {
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
    let mergedCount = 0
    let skipped = 0
    let accumulated = 0
    let dedupedDuplicate = 0

    for (const payload of batch) {
      if (payload.instanceId !== instanceId) {
        continue
      }
      const incoming = liveItemFor(payload, baseSeq)

      if (!incoming) {
        skipped += 1
        continue
      }

      // Per-(turnId, kind) accumulator for AgentText / AgentThought.
      // The wire ships one SeqTranscriptItem per streamed chunk; the
      // snapshot-timeline projector folded these at render time, but
      // duplicate live events (transient WS dispatcher glitch, actor
      // recovery re-emit) leaked through the projector unchanged and
      // surfaced as the same message rendered 3x. Captain's call:
      // collapse at the cache layer using `turnId` as the primary
      // key, then dedup-by-`endsWith` on the accumulated text so a
      // duplicate chunk arriving after the accumulator has the
      // original text is dropped before it can compound.
      const kind = incoming.item.kind

      if (kind === TranscriptItemKind.AgentText || kind === TranscriptItemKind.AgentThought) {
        const idx = findAccumulatorIndex(nextItems, kind, incoming.turnId)

        if (idx >= 0) {
          const existing = nextItems[idx]!
          const existingText = (existing.item as { text?: string }).text ?? ''
          const incomingText = (incoming.item as { text?: string }).text ?? ''

          // `endsWith` dedup: if the existing accumulator already
          // ends with the incoming chunk's exact text, the chunk is
          // a duplicate of what we just appended (either same-batch
          // or rapid re-fire across batches). Concatenating again
          // would render the text twice. Skip — including the seq
          // update so the dedup is invisible to downstream
          // bookkeeping.
          if (incomingText.length > 0 && existingText.endsWith(incomingText)) {
            dedupedDuplicate += 1
            continue
          }
          // Replace in place with the concatenated text. Cloning
          // the SeqTranscriptItem (vs mutating the cached entry)
          // keeps Vue's reactivity tracking honest — vue-query
          // diffs by object reference at the page level; in-place
          // mutation would silently desync subscribers that read
          // `items[i].item.text` through a computed.
          nextItems[idx] = {
            seq: existing.seq,
            turnId: existing.turnId ?? incoming.turnId,
            item: { ...existing.item, text: existingText + incomingText } as typeof existing.item
          }
          accumulated += 1
          // Track lastSeq from the most recent chunk even though
          // we didn't push, so the head's latestSeq advances and
          // the projector's `updatedAt = it.seq` derivation reads
          // the freshest position.
          baseSeq += 1
          lastSeq = incoming.seq
          continue
        }
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
    pendingByInstance.delete(instanceId)

    // No-op short-circuit when no item changed. Accumulator hits
    // count too — those mutate `nextItems[idx]` and produce a new
    // page object, so vue-query needs the new reference to notify
    // subscribers. mergeToolCallUpdate similarly mutates entries.
    if (applied === 0 && accumulated === 0 && mergedCount === 0) {
      return old
    }

    if (dedupedDuplicate > 0) {
      log.warn('transcript-patcher: dropped duplicate text via endsWith accumulator', {
        instanceId,
        batchSize: batch.length,
        dedupedDuplicate
      })
    }

    log.trace('transcript-patcher.batch-applied', {
      instanceId,
      batchSize: batch.length,
      applied,
      merged: mergedCount,
      accumulated,
      skipped,
      dedupedDuplicate,
      headItemCount: nextItems.length
    })

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
  // Track the daemon's truth so a reconnect can ask for everything
  // strictly newer. When the wire carries `seq` (current daemon) the
  // remote-bridge's `setRemoteResyncHandler` reads this on reconnect
  // and dispatches `instance_snapshot_chat { after }`. Older daemons
  // leave the field undefined; reconnect falls back to a head fetch.
  recordLastSeenSeq(payload.instanceId, payload.seq)
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

/// Max items the delta-replay path will pull per instance per
/// reconnect. Picked at 500 because the mirror's ring buffer caps at
/// 5000, and 500 covers minutes of streaming at typical agent
/// throughput (~30 chunks/s burst, sub-1/s sustained). When the
/// daemon has more than 500 newer-than-cursor items, the response's
/// `hasMore` stays `true` — the patcher then logs and stops, leaving
/// the captain a one-page-stale view; a manual scroll-to-bottom +
/// next live event closes the gap. For longer offline windows the
/// captain can refresh manually.
const DELTA_REPLAY_PAGE_SIZE = 500

/// Apply a daemon-served delta page (items strictly newer than the
/// caller's last-seen seq) onto the head of the cached chat infinite
/// query. Items are sequenced by the daemon — we trust their `seq`
/// values and merge ToolCallUpdate entries onto in-cache ToolCall
/// rows the same way the live patcher does, so a delta-replay that
/// catches a mid-streaming tool call collapses correctly instead of
/// stacking a phantom row.
function applyChatDeltaPage(queryClient: QueryClient, instanceId: string, items: SeqTranscriptItem[]): number {
  if (items.length === 0) {
    return 0
  }
  let applied = 0

  queryClient.setQueryData<PatchableInfiniteData>(['snapshot-chat', instanceId], (old) => {
    if (!old || old.pages.length === 0) {
      // No cached pages yet — first hydration hasn't landed. The
      // boot path will fetch a fresh head page anyway; drop the
      // delta to avoid double-painting.
      return old
    }
    const head = old.pages[0]

    if (!head) {
      return old
    }
    const nextItems = [...head.items]
    let lastSeq = head.latestSeq ?? 0

    for (const incoming of items) {
      const merged = mergeToolCallUpdate(nextItems, incoming)

      if (!merged) {
        nextItems.push(incoming)
      }

      if (incoming.seq > lastSeq) {
        lastSeq = incoming.seq
      }
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

  return applied
}

async function replayDeltaForInstance(queryClient: QueryClient, instanceId: string): Promise<void> {
  const after = lastSeenSeqByInstance.get(instanceId)

  if (after === undefined) {
    return
  }

  try {
    const snap = (await invoke(TauriCommand.InstanceSnapshotChat, {
      instanceId,
      after,
      limit: DELTA_REPLAY_PAGE_SIZE
    })) as ChatSnapshot

    const applied = applyChatDeltaPage(queryClient, instanceId, snap.items)

    log.trace('snapshot.delta-replay.applied', {
      instanceId,
      after,
      received: snap.items.length,
      applied,
      hasMore: snap.hasMore,
      latestSeq: snap.latestSeq
    })

    if (snap.hasMore) {
      log.warn(
        'transcript-patcher: delta-replay page exhausted with more items waiting — captain may need to refresh',
        {
          instanceId,
          after,
          pageSize: DELTA_REPLAY_PAGE_SIZE
        }
      )
    }
  } catch(err) {
    log.warn('transcript-patcher: delta-replay failed', { instanceId, after }, err)
    throw err
  }
}

/**
 * Top-level resync hook for the remote bridge. Called on silent
 * reauth after a dropped WS. For every instance we've ever seen on
 * this page, pulls a delta page (`instance_snapshot_chat { after }`)
 * and patches the head. Meta / terminals / queue / instances list
 * stay current via vue-query invalidations — UI subscribers refetch
 * automatically.
 *
 * Returns `true` when the resync completed without falling back to
 * the page reload. `false` signals the caller (`remote-bridge.ts`)
 * to reload the page as a coarse-but-correct safety net.
 */
export async function resyncFromRemote(queryClient: QueryClient): Promise<boolean> {
  const instanceIds = [...lastSeenSeqByInstance.keys()]

  if (instanceIds.length === 0) {
    // No prior state to delta-replay against — the page must be in
    // an unusual state (rare reconnect before any event ever
    // landed). Fall back to reload.
    return false
  }

  try {
    await Promise.all(instanceIds.map((id) => replayDeltaForInstance(queryClient, id)))
  } catch {
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
