/**
 * Per-instance FIFO submit queue. The composer routes
 * "submit while phase != idle" through `pushToQueue` instead of
 * dispatching immediately; captain drains the head explicitly via
 * the `queue.send` keybind (Ctrl+Enter by default) or the per-row
 * "send now" / "drop" buttons on the queue strip. The queue never
 * auto-dispatches on turn end — captain stays in control. Cancel of
 * the in-flight turn (`stopReason === 'cancelled'`) still flushes
 * the queue alongside the cancelled head, matching pilot.py.
 *
 * Storage shape carries both composer image pills (`pills`, for the
 * queue strip preview) and skill attachments (`skillAttachments`,
 * dispatched as ACP `ContentBlock::Resource` entries). Snapshotting at
 * enqueue time means a downstream skill-body edit doesn't change what
 * the queued turn sends — pick again to refresh.
 */

import { computed, reactive, type ComputedRef } from 'vue'

import { nextSeq } from './sequence'
import { onTurnEnded, type TurnEndedRaw } from './use-turns'
import { useActiveInstance, type InstanceId } from '../chrome/use-active-instance'
import { useAdapter } from '../chrome/use-adapter'
import { pushToast } from '../ui-state/use-toasts'
import { ToastTone, type ComposerPill } from '@components'
import { type Attachment } from '@ipc'
import { log } from '@lib'

export interface QueuedItem {
  id: string
  text: string
  pills: ComposerPill[]
  skillAttachments: Attachment[]
  enqueuedAt: number
}

export type QueuedItemInput = Omit<QueuedItem, 'id' | 'enqueuedAt'>

interface QueueState {
  items: QueuedItem[]
}

const states = reactive(new Map<InstanceId, QueueState>())

function slotFor(id: InstanceId): QueueState {
  let slot = states.get(id)

  if (!slot) {
    slot = { items: [] }
    states.set(id, slot)
  }

  return slot
}

/** Append to the tail; returns the persisted entry (id + enqueuedAt populated). */
export function pushToQueue(id: InstanceId, item: QueuedItemInput): QueuedItem {
  const slot = slotFor(id)
  const seq = nextSeq(id)
  const queued: QueuedItem = {
    id: crypto.randomUUID(),
    text: item.text,
    pills: item.pills,
    skillAttachments: item.skillAttachments,
    enqueuedAt: seq
  }

  slot.items.push(queued)

  return queued
}

/** Pop the head; returns the popped entry or `undefined` when empty. */
export function popQueueHead(id: InstanceId): QueuedItem | undefined {
  const slot = states.get(id)

  if (!slot || slot.items.length === 0) {
    return undefined
  }

  return slot.items.shift()
}

/**
 * Pop a specific entry by id; returns the entry + its original
 * position so callers can re-insert at the same slot (edit
 * round-trip). `undefined` when the id isn't present.
 */
export function popQueueItem(id: InstanceId, itemId: string): { item: QueuedItem; position: number } | undefined {
  const slot = states.get(id)

  if (!slot) {
    return undefined
  }
  const idx = slot.items.findIndex((q) => q.id === itemId)

  if (idx === -1) {
    return undefined
  }
  const [item] = slot.items.splice(idx, 1)

  return { item, position: idx }
}

/**
 * Insert at a specific slot, clamped to `[0, items.length]`. Used
 * by the queue-edit round-trip: the captain pops an entry into the
 * composer (`popQueueItem`), edits, then re-submits — the resubmit
 * lands the entry back at its original position so queue order is
 * preserved.
 */
export function pushToQueueAt(id: InstanceId, position: number, item: QueuedItemInput): QueuedItem {
  const slot = slotFor(id)
  const seq = nextSeq(id)
  const queued: QueuedItem = {
    id: crypto.randomUUID(),
    text: item.text,
    pills: item.pills,
    skillAttachments: item.skillAttachments,
    enqueuedAt: seq
  }
  const at = Math.max(0, Math.min(position, slot.items.length))

  slot.items.splice(at, 0, queued)

  return queued
}

/** Remove a specific entry by id; no-op when not present. */
export function removeFromQueue(id: InstanceId, itemId: string): void {
  const slot = states.get(id)

  if (!slot) {
    return
  }
  slot.items = slot.items.filter((q) => q.id !== itemId)
}

/** Cancel-flush: drop every queued item for this instance. */
export function flushQueue(id: InstanceId): void {
  const slot = states.get(id)

  if (!slot) {
    return
  }
  slot.items = []
}

/** Teardown: drop the slot entirely. Pairs with other `reset*` helpers. */
export function resetQueue(id: InstanceId): void {
  states.delete(id)
}

/** Test-only: clear every instance's queue. */
export function __resetAllQueues(): void {
  states.clear()
}

export function useQueue(instanceId?: InstanceId): {
  items: ComputedRef<QueuedItem[]>
  enqueue: (item: QueuedItemInput) => QueuedItem | undefined
  flush: () => void
} {
  const { id: activeId } = useActiveInstance()

  const items = computed<QueuedItem[]>(() => {
    const resolved = instanceId ?? activeId.value

    if (!resolved) {
      return []
    }

    return states.get(resolved)?.items ?? []
  })

  function enqueue(item: QueuedItemInput): QueuedItem | undefined {
    const resolved = instanceId ?? activeId.value

    if (!resolved) {
      return undefined
    }

    return pushToQueue(resolved, item)
  }

  function flush(): void {
    const resolved = instanceId ?? activeId.value

    if (!resolved) {
      return
    }
    flushQueue(resolved)
  }

  return {
    items,
    enqueue,
    flush
  }
}

/**
 * Cancel-flush stop reason. ACP wire string — see
 * `agent-client-protocol`'s `StopReason` (snake_case).
 */
const FLUSH_STOP_REASON = 'cancelled'

function onTurnEndedRoute(id: InstanceId, raw: TurnEndedRaw): void {
  if (raw.stopReason !== FLUSH_STOP_REASON) {
    return
  }
  const slot = states.get(id)
  const dropped = slot?.items.length ?? 0

  flushQueue(id)

  if (dropped > 0) {
    log.info('queue flushed on cancel', { instanceId: id, dropped })
    pushToast(ToastTone.Warn, 'queue cleared')
  }
}

/**
 * Submit a queued item via the adapter. Shared by the keybind /
 * per-row dispatch paths so error-toast + log copy stay uniform.
 */
function submitQueuedItem(id: InstanceId, item: QueuedItem): void {
  const { submit } = useAdapter()

  log.info('queue dispatch', {
    instanceId: id,
    queuedItemId: item.id,
    textLen: item.text.length
  })
  void submit({
    text: item.text,
    instanceId: id,
    attachments: item.skillAttachments
  }).catch((err) => {
    log.error('queue dispatch failed', { instanceId: id, queuedItemId: item.id }, err)
    pushToast(ToastTone.Err, `queue dispatch failed: ${String(err)}`)
  })
}

/** Pop + submit the head. No-op when empty. */
export function dispatchQueueHead(id: InstanceId): void {
  const head = popQueueHead(id)

  if (!head) {
    return
  }
  submitQueuedItem(id, head)
}

/** Pop + submit a specific entry; the rest of the queue keeps its order. */
export function dispatchQueueItem(id: InstanceId, itemId: string): void {
  const popped = popQueueItem(id, itemId)

  if (!popped) {
    return
  }
  submitQueuedItem(id, popped.item)
}

let queueDispatcherStop: (() => void) | undefined

/**
 * Wire the cancel-flush watcher to the turn-ended signal.
 * Idempotent. The queue never auto-dispatches; this subscription
 * exists purely to drop queued items when the in-flight turn was
 * cancelled by the user.
 */
export function startQueueDispatcher(): void {
  if (queueDispatcherStop) {
    return
  }
  queueDispatcherStop = onTurnEnded(onTurnEndedRoute)
}

export function stopQueueDispatcher(): void {
  queueDispatcherStop?.()
  queueDispatcherStop = undefined
}
