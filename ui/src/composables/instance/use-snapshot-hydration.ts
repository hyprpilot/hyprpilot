/**
 * Snapshot → store hydration (Phase C2 follow-up).
 *
 * `useTurns` is populated by the live event router (`use-session-stream`)
 * for events that arrive after mount. Anything that streamed BEFORE
 * the UI subscribed (mid-session reconnect, focus-switch into an
 * instance, remote bridge that just authenticated) is invisible to
 * the live path — the daemon emits events into a `tokio::broadcast`
 * with no replay.
 *
 * This hook closes that gap. It watches the per-instance
 * `MetaSnapshot.turns` field — populated by the daemon mirror on
 * every `TurnStarted` / `TurnEnded` / `UsageUpdate` it observes —
 * and replays each record into the `useTurns` store via the same
 * `pushTurnStarted` / `pushTurnEnded` / `pushUsageUpdate` mutation
 * surface the live router uses. Result: the chat header's elapsed +
 * usage chips render against the daemon's truth even when the live
 * stream missed every event in the session's history.
 *
 * Hydration is idempotent: each push is guarded against existing
 * records inside `use-turns.ts` (it's also idempotent there for
 * replay safety), and `pushUsageUpdate` overwrites the prior reading.
 *
 * Run this composable wherever you need a snapshot-driven view of
 * `useTurns` — today the chat viewport's `Overlay.vue` mounts it
 * once per active instance.
 */

import { watch, type ComputedRef } from 'vue'

import { useInstanceMetaQuery } from './use-instance-meta-query'
import { pushTurnEnded, pushTurnStarted, pushUsageUpdate } from './use-turns'
import { type InstanceId } from '../chrome/use-active-instance'
import { type TurnSnapshot } from '@ipc'
import { log } from '@lib'

export interface UseSnapshotHydrationApi {
  /** Stop watching — wired automatically when the host component unmounts. */
  stop: () => void
}

/**
 * Hydrate `useTurns` for `instanceId` from the latest snapshot read.
 * The instance ref drives both the meta query (re-keys on change) AND
 * the push targets — switching instances replays into the new
 * instance's slot.
 */
export function useSnapshotHydration(instanceId: ComputedRef<InstanceId | undefined>): UseSnapshotHydrationApi {
  const meta = useInstanceMetaQuery(instanceId)

  // Track which (instanceId, turnId) pairs we've replayed so a
  // subsequent meta refresh that re-ships the same turn record doesn't
  // re-push a duplicate `TurnStarted`. The store-side `pushTurnStarted`
  // is itself idempotent on `id` (skips on existing) so this is
  // belt-and-braces; the explicit set also avoids the linear scan
  // through `slot.turns` on every refresh.
  const seenTurnStarts = new Set<string>()
  const seenTurnEnds = new Set<string>()

  const stopHandle = watch(
    () => {
      const data = meta.data.value
      const id = instanceId.value

      if (id === undefined || !data) {
        return undefined
      }

      return { id, turns: data.turns ?? [] }
    },
    (snap) => {
      if (!snap) {
        return
      }
      const { id, turns } = snap

      log.trace('snapshot.hydrate.meta-arrived', {
        instanceId: id,
        turnCount: turns.length
      })

      let pushed = 0
      let skipped = 0

      for (const t of turns) {
        const isNewStart = !seenTurnStarts.has(`${id}::${t.id}`)
        const isNewEnd = t.endedAtMs !== undefined && !seenTurnEnds.has(`${id}::${t.id}`)

        applyTurnSnapshot(id, t, seenTurnStarts, seenTurnEnds)

        if (isNewStart || isNewEnd) {
          pushed += 1
        } else {
          skipped += 1
        }
      }

      log.trace('snapshot.hydrate.applied', {
        instanceId: id,
        pushed,
        skipped
      })
    },
    { immediate: true }
  )

  // Reset the dedup sets on instance change — a new instance has its
  // own turn ids (UUIDs are unique, but resetting keeps the set
  // bounded across long sessions where the captain rotates instances).
  watch(instanceId, (next, prev) => {
    log.trace('snapshot.hydrate.instance-flip', { from: prev, to: next })
    seenTurnStarts.clear()
    seenTurnEnds.clear()
  })

  return {
    stop: () => stopHandle()
  }
}

function applyTurnSnapshot(instanceId: InstanceId, t: TurnSnapshot, seenStarts: Set<string>, seenEnds: Set<string>): void {
  const startKey = `${instanceId}::${t.id}`

  if (!seenStarts.has(startKey)) {
    seenStarts.add(startKey)
    pushTurnStarted(instanceId, {
      turnId: t.id,
      sessionId: t.sessionId,
      startedAtMs: t.startedAtMs
    })
  }

  if (t.usage) {
    pushUsageUpdate(instanceId, t.sessionId, t.id, {
      used: t.usage.used,
      size: t.usage.size,
      cost: t.usage.cost
    })
  }

  if (t.endedAtMs !== undefined && !seenEnds.has(startKey)) {
    seenEnds.add(startKey)
    pushTurnEnded(instanceId, {
      turnId: t.id,
      sessionId: t.sessionId,
      stopReason: t.stopReason,
      endedAtMs: t.endedAtMs
    })
  }
}
