/**
 * Per-instance FIFO submit queue. The composer routes
 * "submit while phase != idle" through `pushToQueue` instead of
 * dispatching immediately; the K-260 turn-end watcher in
 * `useQueueDispatcher` drains the head when the matching turn lands
 * `acp:turn-ended` with `stopReason === 'end_turn'`. Cancel-flush:
 * `stopReason === 'cancelled'` discards every queued item alongside
 * the in-flight head — pilot.py-equivalent semantics.
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
 * Stop reasons that drain the queue head (vs. flush it). ACP wire
 * strings — see `agent-client-protocol`'s `StopReason` (snake_case).
 */
const DISPATCH_STOP_REASON = 'end_turn'
const FLUSH_STOP_REASON = 'cancelled'

function onTurnEndedRoute(id: InstanceId, raw: TurnEndedRaw): void {
  if (raw.stopReason === FLUSH_STOP_REASON) {
    const slot = states.get(id)
    const dropped = slot?.items.length ?? 0

    flushQueue(id)

    if (dropped > 0) {
      log.info('queue flushed on cancel', { instanceId: id, dropped })
      pushToast(ToastTone.Warn, 'queue cleared')
    }

    return
  }

  if (raw.stopReason !== DISPATCH_STOP_REASON) {
    return
  }
  const head = popQueueHead(id)

  if (!head) {
    return
  }
  const { submit } = useAdapter()

  log.info('queue dispatch', {
    instanceId: id,
    queuedItemId: head.id,
    textLen: head.text.length
  })
  void submit({
    text: head.text,
    instanceId: id,
    attachments: head.skillAttachments
  }).catch((err) => {
    log.error('queue dispatch failed', { instanceId: id, queuedItemId: head.id }, err)
    pushToast(ToastTone.Err, `queue dispatch failed: ${String(err)}`)
  })
}

let queueDispatcherStop: (() => void) | undefined

/**
 * Wire the queue dispatcher to the turn-ended signal. Idempotent —
 * subsequent calls reuse the existing subscription. Pair with
 * `stopQueueDispatcher` for teardown (test cleanup, app unmount).
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
