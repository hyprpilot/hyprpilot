import { computed, reactive, type ComputedRef } from 'vue'

import { type PermissionPrompt } from '@components'
import { invoke, TauriCommand, type PermissionOptionView } from '@ipc'

import { nextSeq } from './sequence'
import { useActiveInstance, type InstanceId } from './useActiveInstance'

// Stored shape — `queued` is derived at read time in the `pending`
// computed (oldest-by-createdAt is active, everything else queued), not
// snapshotted at insert. Snapshotting desyncs after the first entry
// gets evicted: remaining rows would keep `queued: true` and the
// stack's activeId lookup (`prompts.find((p) => !p.queued)`) goes
// undefined — buttons disappear.
export interface PendingPermission extends Omit<PermissionPrompt, 'queued'> {
  instanceId: InstanceId
  requestId: string
  sessionId: string
  createdAt: number
}

export interface PermissionsState {
  pending: Map<string, PendingPermission>
}

export interface PermissionRequestRaw {
  request_id: string
  tool: string
  kind?: string
  args?: string
  options: PermissionOptionView[]
}

export enum PermissionDecision {
  Allow = 'allow',
  Deny = 'deny'
}

const states = reactive(new Map<InstanceId, PermissionsState>())

function slotFor(id: InstanceId): PermissionsState {
  let slot = states.get(id)
  if (!slot) {
    slot = { pending: new Map() }
    states.set(id, slot)
  }

  return slot
}

/**
 * Accumulates a pending permission prompt for the given instance.
 * Keyed by `request_id`; re-pushing the same id replaces the slot.
 */
export function pushPermissionRequest(id: InstanceId, sessionId: string, raw: PermissionRequestRaw): void {
  const slot = slotFor(id)
  const seq = nextSeq(id)
  slot.pending.set(raw.request_id, {
    instanceId: id,
    requestId: raw.request_id,
    sessionId,
    id: raw.request_id,
    tool: raw.tool,
    kind: raw.kind ?? 'acp',
    args: raw.args ?? '',
    createdAt: seq
  })
}

/**
 * Removes a pending entry. Called today only on successful reply.
 * TODO(K-245): also evict on a daemon 'permission-cancelled' broadcast
 * once the PermissionController emits one.
 */
export function evictPermission(id: InstanceId, requestId: string): void {
  states.get(id)?.pending.delete(requestId)
}

export function resetPermissions(id: InstanceId): void {
  states.delete(id)
}

export interface PendingPermissionView extends PendingPermission {
  queued: boolean
}

export function usePermissions(instanceId?: InstanceId): {
  pending: ComputedRef<PendingPermissionView[]>
  allow: (requestId: string) => Promise<void>
  deny: (requestId: string) => Promise<void>
} {
  const { id: activeId } = useActiveInstance()

  const pending = computed<PendingPermissionView[]>(() => {
    const resolved = instanceId ?? activeId.value
    if (!resolved) {
      return []
    }
    const state = states.get(resolved)
    if (!state) {
      return []
    }
    const sorted = Array.from(state.pending.values()).sort((a, b) => a.createdAt - b.createdAt)

    return sorted.map((p, i) => ({ ...p, queued: i > 0 }))
  })

  async function respond(requestId: string, decision: PermissionDecision): Promise<void> {
    const resolved = instanceId ?? activeId.value
    if (!resolved) {
      throw new Error('no active instance')
    }
    const entry = states.get(resolved)?.pending.get(requestId)
    if (!entry) {
      throw new Error(`no pending permission request ${requestId}`)
    }
    // K-245's PermissionController finalises the option-id shape; today
    // `permission_reply` accepts raw decision strings and maps them.
    await invoke(TauriCommand.PermissionReply, {
      sessionId: entry.sessionId,
      requestId: entry.requestId,
      optionId: decision
    })
    evictPermission(resolved, requestId)
  }

  return {
    pending,
    allow: (requestId) => respond(requestId, PermissionDecision.Allow),
    deny: (requestId) => respond(requestId, PermissionDecision.Deny)
  }
}
