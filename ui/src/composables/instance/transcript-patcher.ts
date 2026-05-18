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
  listen,
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

/// Build a stable identity for a payload — `undefined` when the
/// item kind doesn't warrant in-batch dedup (tool-call updates carry
/// their own state-machine merging; tiny chunks of text legitimately
/// repeat). `AgentText` / `AgentThought` over the
/// `DUP_DEDUP_MIN_TEXT_CHARS` threshold are the meaningful candidates:
/// the daemon's chunk boundaries are arbitrary mid-word but a full
/// sentence/paragraph landing twice in the same flush burst is almost
/// always a wire-side mistake.
const DUP_DEDUP_MIN_TEXT_CHARS = 24

function duplicateSignature(payload: TranscriptEventPayload): string | undefined {
  const item = payload.item

  if (item.kind === TranscriptItemKind.AgentText || item.kind === TranscriptItemKind.AgentThought) {
    const text = item.text ?? ''

    if (text.length < DUP_DEDUP_MIN_TEXT_CHARS) {
      return undefined
    }

    return `${item.kind}:${payload.turnId ?? ''}:${text}`
  }

  return undefined
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
    let dedupedDuplicate = 0

    // Defensive in-batch content dedup. Captain reported the same
    // agent-text message landing in the chat 3 times — the daemon's
    // broadcast is single-fire per chunk so the duplication is
    // either a transient browser-side listener-dispatch glitch on
    // remote-WS reconnect OR the actor genuinely re-emitting in
    // some recovery path. Either way, identical TranscriptItem
    // payloads (same turnId + same item shape) inside one flush
    // batch are a strong duplicate signal — legitimate streaming
    // chunks are rarely byte-identical at >50 chars. Dedup the
    // batch up-front so the cache only ever sees the first
    // occurrence.
    const seenSignatures = new Set<string>()
    const filtered: TranscriptEventPayload[] = []

    for (const payload of batch) {
      if (payload.instanceId !== instanceId) {
        continue
      }
      const sig = duplicateSignature(payload)

      if (sig !== undefined) {
        if (seenSignatures.has(sig)) {
          dedupedDuplicate += 1
          continue
        }
        seenSignatures.add(sig)
      }
      filtered.push(payload)
    }

    for (const payload of filtered) {
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
    pendingByInstance.delete(instanceId)

    if (applied === 0) {
      return old
    }

    if (dedupedDuplicate > 0) {
      log.warn('transcript-patcher: dropped duplicate payloads in batch', {
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
  // with the per-instance composable's monotonic counter. The wire
  // doesn't ship the daemon's seq on live events; this is just a local
  // ordinal used for diagnostic traces.
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
  flushScheduled = false
  started = false
}

/**
 * Test-only reset. Drops every listener + clears the pending queue.
 */
export function __resetTranscriptPatcherForTests(): void {
  stopTranscriptPatcher()
}
