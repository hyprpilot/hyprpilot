import { computed, reactive, type ComputedRef } from 'vue'

import { Phase } from '@components'

import { InstanceState } from '@ipc'
import { useActiveInstance, type InstanceId } from './use-active-instance'
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
 *   1. awaiting  ← a pending permission prompt exists
 *   2. pending   ← any tool call has no terminal status
 *   3. streaming ← instance is running, a turn is currently open, and
 *                  the agent has produced at least one chunk in the
 *                  transcript (any past turn counts — distinguishes
 *                  "agent has begun replying" vs the pre-reply gap)
 *   4. working   ← instance is running and a turn is open but no agent
 *                  chunks yet (sent prompt, awaiting first chunk)
 *   5. idle      ← default — including the in-between-turns state where
 *                  the session is alive but no turn is open. Composer
 *                  dispatches in `idle`; routes to queue otherwise.
 *
 * Gating busy phases on `openTurnId` (vs. the previous "any agent turn
 * exists in the transcript") is the K-281 fix for the queue-stuck bug:
 * once the agent had spoken once, phase used to stay `streaming`
 * forever, so submits routed to the queue and the queue dispatcher
 * (which only fires on `acp:turn-ended`) had no future event to
 * trigger a drain.
 */
export function usePhase(instanceId?: InstanceId): { phase: ComputedRef<Phase> } {
  const { id: activeId } = useActiveInstance()
  const resolved = computed(() => instanceId ?? activeId.value)

  const phase = computed<Phase>(() => {
    const id = resolved.value
    if (!id) {
      return Phase.Idle
    }

    const { pending } = usePermissions(id)
    if (pending.value.length > 0) {
      return Phase.Awaiting
    }

    const { calls } = useTools(id)
    const hasRunningTool = calls.value.some((c) => {
      const s = (c.status ?? '').toLowerCase()
      return s !== 'completed' && s !== 'done' && s !== 'failed' && s !== 'error'
    })
    if (hasRunningTool) {
      return Phase.Pending
    }

    const sig = signals.get(id)
    const { openTurnId } = useTurns(id)
    if (sig?.runtimeState === InstanceState.Running && openTurnId.value) {
      const { turns } = useTranscript(id)
      const hasAgentTurn = turns.value.some((t) => t.role === TurnRole.Agent)
      if (hasAgentTurn) {
        return Phase.Streaming
      }
      return Phase.Working
    }

    return Phase.Idle
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
