import { computed, reactive, type ComputedRef } from 'vue'

import { usePermissions } from './use-permissions'
import { useTools } from './use-tools'
import { TurnRole, useTranscript } from './use-transcript'
import { useTurns } from './use-turns'
import { useActiveInstance, type InstanceId } from '../chrome/use-active-instance'
import { Phase } from '@components'
import { InstanceState } from '@ipc'

interface PhaseSignals {
  runtimeState?: InstanceState
}

const signals = reactive(new Map<InstanceId, PhaseSignals>())

/**
 * Push the latest observed `InstanceState` for `id`. Wired from
 * `use-session-stream`'s `acp:instance-state` listener so the
 * runtime view of "is this instance alive?" stays current for
 * consumers that need it (chrome title accent, palette lifecycle
 * indicators, etc.). NOTE: phase derivation no longer reads this
 * — see `usePhase` for why.
 */
export function pushInstanceState(id: InstanceId, state: InstanceState): void {
  let slot = signals.get(id)

  if (!slot) {
    slot = {}
    signals.set(id, slot)
  }
  slot.runtimeState = state
}

/// Read the last-known runtimeState for an instance. `undefined`
/// when no state event has landed yet. Consumed by the chrome /
/// palette to colour "this instance is alive vs dead" affordances
/// without gating the composer's stop button on the same signal
/// (which produced a race on remote fresh spawns).
export function runtimeStateFor(id: InstanceId): InstanceState | undefined {
  return signals.get(id)?.runtimeState
}

/**
 * Computes the overlay phase for an instance.
 *
 * Decision ladder (first-matching wins):
 *   1. awaiting  ← a pending permission prompt exists (live, not replayed)
 *   2. *busy*    ← a turn is currently open. Sub-classified:
 *                    - working   if any tool call is non-terminal
 *                    - streaming if the agent has emitted a chunk
 *                    - working   otherwise (sent prompt, no chunks yet)
 *   3. idle      ← default — including the in-between-turns state where
 *                  the session is alive but no turn is open. Composer
 *                  dispatches in `idle`; routes to queue otherwise.
 *
 * `openTurnId` is the sole "we're busy" gate. Replays don't fire
 * `acp:turn-started` (only a live `Prompt` does), so on `session/load`
 * / `session/fork` `openTurnId` stays undefined and phase correctly
 * resolves to Idle.
 *
 * **No runtimeState AND-gate here** — an earlier shape gated on
 * `runtimeState === Running` AND `openTurnId`, but the gate was
 * redundant (the openTurnId check already filters replays since
 * those never emit TurnStarted) AND racy on remote-host fresh
 * spawns: the actor's `TurnStarted` broadcast lands ~50ms before
 * the `InstanceState::Running` event on the WS, so the AND-gate
 * flipped Idle for that window and the captain's brand-new turn
 * never showed the stop button. The `signals` map + `runtimeStateFor`
 * are kept for other consumers (lifecycle title tint, palette
 * dead-instance dim); they just don't drive phase anymore.
 */
export function usePhase(instanceId?: InstanceId): { phase: ComputedRef<Phase> } {
  const { id: activeId } = useActiveInstance()
  const resolved = computed(() => instanceId ?? activeId.value)

  // S3 — sub-composable refs lifted to factory time. Each sub-composable
  // creates a new `computed()` per call; previously these were invoked
  // inside the `phase` computed body, causing N allocations per reactive
  // read. Lifted version creates them once; sub-composables track
  // active-id changes through their own internal `computed`s.
  const { rowQueue, modalQueue } = usePermissions(instanceId)
  const { runningCount } = useTools(instanceId)
  const { openTurnId } = useTurns(instanceId)
  const { turns } = useTranscript(instanceId)

  const phase = computed<Phase>(() => {
    const id = resolved.value

    if (!id) {
      return Phase.Idle
    }

    if (rowQueue.value.length > 0 || modalQueue.value.length > 0) {
      return Phase.Awaiting
    }

    if (!openTurnId.value) {
      return Phase.Idle
    }

    // Tool-running shares the working hue with text streaming;
    // Pending (red) is reserved for terminal errors.
    if (runningCount.value > 0) {
      return Phase.Working
    }

    const hasAgentTurn = turns.value.some((t) => t.role === TurnRole.Agent)

    if (hasAgentTurn) {
      return Phase.Streaming
    }

    return Phase.Working
  })

  return { phase }
}

export function resetPhaseSignals(id: InstanceId): void {
  signals.delete(id)
}

export function __resetAllPhaseSignals(): void {
  signals.clear()
}

export { Phase }
