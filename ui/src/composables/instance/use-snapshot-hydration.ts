/**
 * Snapshot → store hydration (Phase C2 follow-up).
 *
 * `useTurns` AND `useSessionInfo` are populated by the live event
 * router (`use-session-stream`) for events that arrive after mount.
 * Anything that streamed BEFORE the UI subscribed (mid-session
 * reconnect, focus-switch into an instance, remote bridge that just
 * authenticated) is invisible to the live path — the daemon emits
 * events into a `tokio::broadcast` with no replay.
 *
 * This hook closes that gap. It watches the per-instance
 * `MetaSnapshot` and replays the relevant fields into both stores:
 *
 * - `MetaSnapshot.turns` → `pushTurnStarted` / `pushTurnEnded` /
 *   `pushUsageUpdate` so the chat header's elapsed + usage chips
 *   render against the daemon's truth.
 * - `MetaSnapshot.{cwd, currentModeId, currentModelId,
 *   availableModes, availableModels, configOptions, mcpsCount,
 *   profileId}` → the matching `use-session-info` setters so the
 *   header chrome (cwd / mode / model pills, mcps count, profile)
 *   reflects the daemon's view immediately on snapshot load —
 *   without this, a remote captain saw stale or empty pills until
 *   the next live event for the instance landed (often only after
 *   a turn fired).
 *
 * Hydration is idempotent: each `pushTurnStarted` is guarded against
 * existing records, the session-info setters are last-write-wins.
 *
 * Run this composable wherever you need a snapshot-driven view of
 * `useTurns` / `useSessionInfo` — today the chat viewport's
 * `Overlay.vue` mounts it once per active instance.
 */

import { watch, type ComputedRef } from 'vue'

import { useInstanceMetaQuery } from './use-instance-meta-query'
import {
  pushConfigOptionsUpdate,
  pushCurrentModeUpdate,
  pushInstanceModeState,
  pushInstanceModelState,
  setInstanceCwd,
  setInstanceMcpsCount,
  setInstanceProfile
} from './use-session-info'
import { pushTurnEnded, pushTurnStarted, pushUsageUpdate } from './use-turns'
import { type InstanceId } from '../chrome/use-active-instance'
import { type MetaSnapshot, type TurnSnapshot } from '@ipc'
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

      return { id, data }
    },
    (snap) => {
      if (!snap) {
        return
      }
      const { id, data } = snap

      applySessionInfoFromMeta(id, data)

      const turns = data.turns ?? []

      log.trace('snapshot.hydrate.meta-arrived', {
        instanceId: id,
        turnCount: turns.length,
        hasCwd: data.cwd !== undefined,
        hasMode: data.currentModeId !== undefined,
        hasModel: data.currentModelId !== undefined,
        mcpsCount: data.mcpsCount
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

/**
 * Push the daemon's per-instance session-info fields into the
 * `useSessionInfo` store. Each field has a dedicated setter so the
 * header chrome's reactive computed signals fan out exactly the way
 * they do for live events. Last-write-wins semantics: a subsequent
 * `pushSessionInfoUpdate` / `pushCurrentModeUpdate` / etc. from the
 * live event router overrides our snapshot value, which is the
 * correct behaviour (live truth beats cached snapshot).
 */
function applySessionInfoFromMeta(instanceId: InstanceId, data: MetaSnapshot): void {
  if (data.cwd != null) {
    setInstanceCwd(instanceId, data.cwd)
  }

  if (data.profileId != null) {
    setInstanceProfile(instanceId, data.profileId)
  }
  // mcpsCount is a number on the wire — not optional. Default to 0
  // when the daemon ever ships it as undefined; the header pill
  // hides on `count == 0` anyway.
  setInstanceMcpsCount(instanceId, data.mcpsCount ?? 0)

  // Spawn-time mode / model state — push the advertised list when
  // present (covers both "list arrived alongside currentId" and
  // "list-only, no current yet" cases). When NEITHER is set we'd
  // be pushing empty noise; skip.
  const hasModes = (data.availableModes?.length ?? 0) > 0
  const hasCurrentMode = data.currentModeId != null

  if (hasModes) {
    pushInstanceModeState(instanceId, {
      currentModeId: data.currentModeId,
      availableModes: data.availableModes ?? []
    })
  } else if (hasCurrentMode && data.currentModeId != null) {
    // No advertised list — push only the current mode overlay. Avoids
    // the duplicate write the prior shape produced (set state with
    // empty list, then set current again on the next branch).
    pushCurrentModeUpdate(instanceId, { currentModeId: data.currentModeId })
  }

  const hasModels = (data.availableModels?.length ?? 0) > 0

  if (hasModels) {
    pushInstanceModelState(instanceId, {
      currentModelId: data.currentModelId,
      availableModels: data.availableModels ?? []
    })
  }

  if (data.configOptions !== undefined) {
    pushConfigOptionsUpdate(instanceId, data.configOptions)
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
