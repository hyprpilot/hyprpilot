import { computed, reactive, type ComputedRef } from 'vue'

import { Phase } from '@components'

import { InstanceState } from '@ipc'
import { useActiveInstance, type InstanceId } from '../chrome/use-active-instance'
import { usePermissions } from './use-permissions'
import { useTools } from './use-tools'
import { TurnRole, useTranscript } from './use-transcript'
import { useTurns } from './use-turns'

interface PhaseSignals {
  runtimeState?: InstanceState
}

const signals = reactive(new Map<InstanceId, PhaseSignals>())

export function pushInstanceState(id: InstanceId, state: InstanceState): void {
  let slot = signals.get(id)
  if (!slot) {
    slot = {}
    signals.set(id, slot)
  }
  slot.runtimeState = state
}

/**
 * Computes the overlay phase for an instance from the typed-store signals
 * landed in K-255 + the instance-state events landed in K-251.
 *
 * Decision ladder (first-matching wins):
 *   1. awaiting  ← a pending permission prompt exists (live, not replayed)
 *   2. *busy*    ← instance is running AND a turn is currently open.
 *                  Sub-classified inside the busy gate:
 *                    - pending   if any tool call is non-terminal
 *                    - streaming if the agent has emitted a chunk
 *                    - working   otherwise (sent prompt, no chunks yet)
 *   3. idle      ← default — including the in-between-turns state where
 *                  the session is alive but no turn is open. Composer
 *                  dispatches in `idle`; routes to queue otherwise.
 *
 * Gating EVERY busy sub-phase on `openTurnId` is the session-restore
 * fix: claude-code-acp's `session/load` replay streams historical
 * `tool_call` updates with their suspended-time status (e.g.
 * `in_progress`, `pending`). Without the `openTurnId` gate, those
 * stale entries kept phase pinned at `pending` forever — composer
 * disabled, no way out — even though no real work was in flight.
 * Replays don't fire `acp:turn-started` (only live `Prompt`s do), so
 * `openTurnId` stays undefined after restore and phase correctly
 * resolves to `idle`.
 *
 * The same gate also fixed the older K-281 queue-stuck bug where
 * phase stuck on `streaming` once the agent had ever spoken.
 */
export function usePhase(instanceId?: InstanceId): { phase: ComputedRef<Phase> } {
  const { id: activeId } = useActiveInstance()
  const resolved = computed(() => instanceId ?? activeId.value)

  // S3 — sub-composable refs lifted to factory time. Each sub-composable
  // creates a new `computed()` per call; previously these were invoked
  // inside the `phase` computed body, causing N allocations per reactive
  // read. Lifted version creates them once; sub-composables track
  // active-id changes through their own internal `computed`s.
  const { pending } = usePermissions(instanceId)
  const { calls } = useTools(instanceId)
  const { openTurnId } = useTurns(instanceId)
  const { turns } = useTranscript(instanceId)

  const phase = computed<Phase>(() => {
    const id = resolved.value
    if (!id) {
      return Phase.Idle
    }

    if (pending.value.length > 0) {
      return Phase.Awaiting
    }

    const sig = signals.get(id)
    if (sig?.runtimeState !== InstanceState.Running || !openTurnId.value) {
      // No open turn → idle, regardless of historical tool-call state.
      return Phase.Idle
    }

    const hasRunningTool = calls.value.some((c) => {
      const s = (c.status ?? '').toLowerCase()
      return s !== 'completed' && s !== 'done' && s !== 'failed' && s !== 'error'
    })
    if (hasRunningTool) {
      return Phase.Pending
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
