/**
 * Per-instance submit queue — UI-side mirror of the daemon's
 * authoritative queue state. Captain-staged prompts live on the
 * daemon (per-instance, served via `queue/*` RPC + `acp:queue-changed`
 * broadcast); this composable keeps a local reactive cache for
 * rendering and routes every mutation through `invoke()` so every
 * connected client (Vue desktop, Vue mobile over WS, hyprpilot-nvim)
 * sees the same state.
 *
 * Hydration: on first observation of an unseen instance id (palette
 * count badge, queue strip render, etc.) the store fires a one-off
 * `QueueList` invoke to seed the cache. From there `applyQueueChanged`
 * (driven by the live router on `acp:queue-changed`) keeps it warm.
 *
 * Enqueue lives on `prompts/send` (auto-routes based on busy state on
 * the daemon side), not here. This composable only exposes the
 * management surface: edit, dispatch, remove, move, clear.
 */

import { computed, reactive, watchEffect, type ComputedRef } from 'vue'

import { useActiveInstance, type InstanceId } from '../chrome/use-active-instance'
import type { QueueItem } from '@interfaces/wire/queue'
import type { Attachment } from '@interfaces/wire/session'
import { invoke, TauriCommand } from '@ipc'
import { log } from '@lib'

interface QueueState {
  items: QueueItem[]
  /// Highest `enqueuedSeq` we've observed for this instance — both
  /// from `acp:queue-changed` broadcasts AND `refreshQueue` snapshot
  /// reads. Acts as a write-watermark: a snapshot reply whose newest
  /// entry's seq is OLDER than this is stale (the broadcast arrived
  /// first with a newer state) and gets dropped. Empty queues update
  /// the watermark on every clear so a snapshot that "missed" the
  /// clear can't resurrect dropped items.
  highWaterSeq: number
}

const states = reactive(new Map<InstanceId, QueueState>())
/// Ids the store has fetched at least once. First observation of an
/// unseen id triggers a one-shot `QueueList` invoke to seed the
/// cache. Subsequent reads serve from the local mirror; live
/// `acp:queue-changed` events keep it warm.
const observed = new Set<InstanceId>()

function slotFor(id: InstanceId): QueueState {
  let slot = states.get(id)

  if (!slot) {
    slot = {
      items: [],
      highWaterSeq: 0
    }
    states.set(id, slot)
  }

  return slot
}

/// Pull the highest `enqueuedSeq` from a list, or `0` for an empty
/// list. Drives the watermark check in `applyQueueChanged`.
function maxSeq(items: QueueItem[]): number {
  let max = 0

  for (const item of items) {
    if (item.enqueuedSeq > max) {
      max = item.enqueuedSeq
    }
  }

  return max
}

/// Wire-listener input. Called by `use-session-stream.ts` on every
/// `acp:queue-changed` event AND by `refreshQueue` on snapshot
/// replies. Full-state replace, idempotent on lossy broadcast (Map
/// .set with the same value is a no-op), and watermark-guarded so a
/// late-arriving snapshot can't overwrite a fresher broadcast: the
/// snapshot is dropped silently when its newest seq is older than
/// what we've already observed. Empty incoming lists are accepted
/// unconditionally (clear is a captain-initiated terminal state).
export function applyQueueChanged(id: InstanceId, items: QueueItem[]): void {
  const slot = slotFor(id)
  const incomingMax = maxSeq(items)

  // Clear / non-empty newer state both go through. The only path we
  // drop is "snapshot whose newest seq is OLDER than ours" — which
  // means we already saw a fresher broadcast and the snapshot would
  // resurrect stale rows.
  if (items.length > 0 && incomingMax < slot.highWaterSeq) {
    log.trace('use-queue: dropped stale snapshot', {
      instanceId: id,
      incomingMax,
      observedMax: slot.highWaterSeq
    })

    return
  }
  slot.items = items
  slot.highWaterSeq = Math.max(slot.highWaterSeq, incomingMax)
  observed.add(id)
}

/// Force-refresh from the daemon. Called on first observation of an
/// unseen instance id (palette badge, focus flip) and after WS
/// reconnect when the SPA reloads its in-memory state. Routes
/// through `instance/snapshot/queue` so the read serves directly
/// from the mirror cache without an actor mailbox round-trip — same
/// pattern the other snapshot endpoints use.
export async function refreshQueue(id: InstanceId): Promise<void> {
  try {
    const { items } = await invoke(TauriCommand.InstanceSnapshotQueue, { instanceId: id })

    applyQueueChanged(id, items)
  } catch(err) {
    log.warn('instance/snapshot/queue failed', { instanceId: id, err: String(err) })
  }
}

/// Reset the local mirror for an instance. Wired into `cleanupInstance`
/// so an ended instance's queue mirror doesn't leak across spawns.
/// The daemon's queue dies with the actor — the mirror just tracks
/// that lifecycle locally.
export function resetQueue(id: InstanceId): void {
  states.delete(id)
  observed.delete(id)
}

/// Test-only escape hatch — same shape as the other instance stores.
export function __resetAllQueues(): void {
  states.clear()
  observed.clear()
}

export interface UseQueueApi {
  /// Live queue for the resolved instance. Empty array when no
  /// instance is resolved or the queue is empty.
  items: ComputedRef<QueueItem[]>
  /// In-place edit. `text` is required; `attachments` undefined keeps
  /// the existing list, `[]` clears them.
  edit: (itemId: string, text: string, attachments?: Attachment[]) => Promise<QueueItem | undefined>
  /// Drop a queued item. No-op when id unknown.
  remove: (itemId: string) => Promise<void>
  /// Captain's "send now" — pops the named item (or head when
  /// omitted) AND dispatches immediately. ACP serialises on the wire
  /// if a turn is in flight.
  dispatch: (itemId?: string) => Promise<void>
  /// Drag-reorder. Position clamps to `[0, len-1]`.
  move: (itemId: string, position: number) => Promise<void>
  /// Drop every item.
  clear: () => Promise<void>
  /// Force-refresh from the daemon. The store fires this automatically
  /// on first observation; callers needing a fresh read (post-error
  /// recovery, manual debug) can also invoke it.
  refresh: () => Promise<void>
}

export function useQueue(instanceId?: InstanceId): UseQueueApi {
  const { id: activeId } = useActiveInstance()

  // First-observation hydration lives in `watchEffect`, not the
  // `items` computed. Computed getters MUST be pure (Vue contract;
  // a side effect inside the getter re-runs on every access). The
  // watchEffect re-fires only when its reactive deps change — here,
  // when `activeId` (and therefore the resolved id) flips. The
  // `observed` set gates concurrent first-observations so two
  // simultaneous `useQueue()` callers (palette badge + queue strip)
  // don't both fire `QueueList`.
  watchEffect(() => {
    const resolved = instanceId ?? activeId.value

    if (!resolved || observed.has(resolved)) {
      return
    }
    observed.add(resolved)
    void refreshQueue(resolved)
  })

  const items = computed<QueueItem[]>(() => {
    const resolved = instanceId ?? activeId.value

    if (!resolved) {
      return []
    }

    return states.get(resolved)?.items ?? []
  })

  async function withResolved<T>(fn: (id: InstanceId) => Promise<T>): Promise<T | undefined> {
    const resolved = instanceId ?? activeId.value

    if (!resolved) {
      return undefined
    }

    return fn(resolved)
  }

  return {
    items,
    edit: (itemId, text, attachments) =>
      withResolved(async(id) => {
        const { item } = await invoke(TauriCommand.QueueEdit, {
          instanceId: id,
          itemId,
          text,
          attachments
        })

        return item
      }),
    remove: (itemId) =>
      withResolved(async(id) => {
        await invoke(TauriCommand.QueueRemove, { instanceId: id, itemId })
      }).then(() => undefined),
    dispatch: (itemId) =>
      withResolved(async(id) => {
        await invoke(TauriCommand.QueueDispatch, { instanceId: id, itemId })
      }).then(() => undefined),
    move: (itemId, position) =>
      withResolved(async(id) => {
        await invoke(TauriCommand.QueueMove, {
          instanceId: id,
          itemId,
          position
        })
      }).then(() => undefined),
    clear: () =>
      withResolved(async(id) => {
        await invoke(TauriCommand.QueueClear, { instanceId: id })
      }).then(() => undefined),
    refresh: () =>
      withResolved(async(id) => {
        await refreshQueue(id)
      }).then(() => undefined)
  }
}
