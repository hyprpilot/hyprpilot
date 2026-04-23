import { computed, reactive, type ComputedRef } from 'vue'

import { Phase } from '@components'

import { InstanceState } from './useSessionStream'
import { useActiveInstance, type InstanceId } from './useActiveInstance'
import { usePermissions } from './usePermissions'
import { useTools } from './useTools'
import { TurnRole, useTranscript } from './useTranscript'

interface PhaseSignals {
  runtimeState: InstanceState | undefined
}

const signals = reactive(new Map<InstanceId, PhaseSignals>())

export function pushInstanceState(id: InstanceId, state: InstanceState): void {
  let slot = signals.get(id)
  if (!slot) {
    slot = { runtimeState: undefined }
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
 *   3. streaming ← instance is running and an agent turn has arrived
 *   4. working   ← instance is running but no agent chunks yet
 *   5. idle      ← default (including null active instance)
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

    const { turns } = useTranscript(id)
    const sig = signals.get(id)
    if (sig?.runtimeState === InstanceState.Running) {
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
