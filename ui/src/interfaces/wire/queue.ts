import type { Attachment } from './session'

/**
 * Daemon-side queue item. Mirrors `src-tauri/src/adapters/queue.rs::QueueItem`.
 * Frontends key off `enqueuedSeq` for ordering (per-instance monotonic, no
 * ties under the actor's single-mailbox lock); `enqueuedAt` is informational
 * (display-only, e.g. "queued 4s ago"). `attachments` carries skill resources
 * + image data — same shape `session_submit` accepts.
 */
export interface QueueItem {
  id: string
  text: string
  attachments?: Attachment[]
  enqueuedSeq: number
  enqueuedAt: number
}

/**
 * Reply shape from `queue/dispatch`. `accepted` flips immediately on
 * actor-accept (not on turn completion) so the UI spinner resolves in
 * milliseconds; the eventual completion arrives via `acp:turn-ended`.
 * `item: undefined` means the queue was empty or the named id was
 * unknown — `accepted` will be `false` in that case.
 */
export interface QueueDispatchResult {
  item?: QueueItem
  sessionId?: string
  accepted: boolean
}

export interface QueueListArgs {
  instanceId?: string
}

export interface QueueListResult {
  items: QueueItem[]
}

export interface QueueEditArgs {
  instanceId?: string
  itemId: string
  text: string
  /** `undefined` keeps existing attachments; `[]` clears them. */
  attachments?: Attachment[]
}

export interface QueueEditResult {
  item: QueueItem
}

export interface QueueRemoveArgs {
  instanceId?: string
  itemId: string
}

export interface QueueRemoveResult {
  removed: boolean
}

export interface QueueMoveArgs {
  instanceId?: string
  itemId: string
  position: number
}

export interface QueueMoveResult {
  moved: boolean
}

export interface QueueClearArgs {
  instanceId?: string
}

export interface QueueClearResult {
  cleared: number
}

export interface QueueDispatchArgs {
  instanceId?: string
  /** `undefined` dispatches the queue head. */
  itemId?: string
}

/**
 * Payload of `acp:queue-changed`. Carries the full post-mutation
 * queue — frontends reconcile by replacement, not by delta.
 */
export interface AcpQueueChangedPayload {
  agentId: string
  instanceId: string
  items: QueueItem[]
}
