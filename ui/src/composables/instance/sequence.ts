/**
 * Per-instance monotonic sequence counter. Every event the demuxer
 * routes consumes a seq via `nextSeq(instanceId)`; each sub-store
 * stamps both `createdAt` (first insert) and `updatedAt` (last merge)
 * from this shared space so the shell can merge turns / stream items /
 * tool calls into one ordered timeline. Stores never maintain their
 * own counter — the demuxer is the sole authority.
 */
import type { InstanceId } from '../chrome/use-active-instance'

const counters = new Map<InstanceId, number>()

export function nextSeq(instanceId: InstanceId): number {
  const current = counters.get(instanceId) ?? 0
  const next = current + 1

  counters.set(instanceId, next)

  return next
}

/** Clears the counter for an instance — pairs with `resetTranscript` / `resetStream` / etc. */
export function resetSeq(instanceId: InstanceId): void {
  counters.delete(instanceId)
}

/** Test-only: clear every counter. */
export function __resetAllSeq(): void {
  counters.clear()
}
