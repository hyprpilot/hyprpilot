import { computed, reactive, type ComputedRef } from 'vue'

import { nextSeq } from './sequence'
import { useActiveInstance, type InstanceId } from '../chrome/use-active-instance'
import { useAgentRegistry } from '../chrome/use-agent-registry'
import { PermissionUi } from '@components'
import { ToolKind } from '@constants/ui'
import type { ToolCallState } from '@constants/wire/transcript'
import type { PermissionView } from '@interfaces/ui'
import type { FormattedToolCall } from '@interfaces/wire/formatted-tool-call'
import { invoke, TauriCommand, type PermissionOptionView } from '@ipc'
import { log, projectFormatted } from '@lib'

/**
 * Stored shape — `queued` is derived at read time on the queue
 * (oldest-by-createdAt is active, everything else queued), not
 * snapshotted at insert. Snapshotting desyncs after eviction: the
 * remaining rows keep `queued: true` and the active-row lookup goes
 * undefined.
 */
export interface PendingPermission {
  instanceId: InstanceId
  agentId: string
  requestId: string
  sessionId: string
  tool: string
  kind: string
  args: string
  rawInput?: Record<string, unknown>
  content: Record<string, unknown>[]
  options: PermissionOptionView[]
  formatted: FormattedToolCall
  createdAt: number
}

export interface PermissionRequestRaw {
  agentId: string
  requestId: string
  tool: string
  kind?: string
  args?: string
  rawInput?: Record<string, unknown>
  content?: Record<string, unknown>[]
  options: PermissionOptionView[]
  formatted: FormattedToolCall
}

// Per-instance pending permissions. `reactive(Map)` matches sibling
// instance-keyed stores (use-queue, use-terminals, use-tools); the
// prior `ref<Record>` + spread-replace pattern allocated a new top-
// level object on every push/evict and required spread-keying every
// inner array too.
const states = reactive(new Map<InstanceId, PendingPermission[]>())

/**
 * Accumulates a pending permission prompt for the given instance.
 * Keyed by `requestId`; re-pushing the same id replaces the entry.
 */
export function pushPermissionRequest(id: InstanceId, sessionId: string, raw: PermissionRequestRaw): void {
  const seq = nextSeq(id)
  const next: PendingPermission = {
    instanceId: id,
    agentId: raw.agentId,
    requestId: raw.requestId,
    sessionId,
    tool: raw.tool,
    kind: raw.kind ?? 'other',
    args: raw.args ?? '',
    rawInput: raw.rawInput,
    content: raw.content ?? [],
    createdAt: seq,
    options: raw.options,
    formatted: raw.formatted
  }
  const current = states.get(id) ?? []
  const filtered = current.filter((p) => p.requestId !== raw.requestId)

  filtered.push(next)
  states.set(id, filtered)
  log.trace('permission pending added', {
    instanceId: id,
    requestId: raw.requestId,
    tool: raw.tool,
    size: filtered.length
  })
}

export function evictPermission(id: InstanceId, requestId: string): void {
  const current = states.get(id)

  if (!current) {
    return
  }
  const filtered = current.filter((p) => p.requestId !== requestId)

  if (filtered.length === current.length) {
    return
  }
  states.set(id, filtered)
  log.trace('permission pending evicted', {
    instanceId: id,
    requestId,
    size: filtered.length
  })
}

export function resetPermissions(id: InstanceId): void {
  states.delete(id)
}

function buildView(p: PendingPermission, queued: boolean): PermissionView {
  const { adapterFor } = useAgentRegistry()
  const call = projectFormatted(p.formatted, {
    id: p.requestId,
    wireName: p.tool,
    kind: (p.kind as ToolKind | undefined) ?? ToolKind.Other,
    state: 'pending' as ToolCallState,
    adapter: adapterFor(p.agentId),
    rawInput: p.rawInput
  })

  return {
    request: {
      requestId: p.requestId,
      instanceId: p.instanceId,
      sessionId: p.sessionId,
      toolName: p.tool
    },
    call,
    options: p.options,
    queued
  }
}

export interface UsePermissionsApi {
  /** Permission requests with `permissionUi: Row` — drives the inline strip. */
  rowQueue: ComputedRef<PermissionView[]>
  /** Permission requests with `permissionUi: Modal` — drives the modal queue. */
  modalQueue: ComputedRef<PermissionView[]>
  /**
   * Resolve a pending permission with the captain's pick. `optionId`
   * must be one of the agent-offered `optionId` values from the
   * pending entry's `options` array. Hyprpilot is transparent to the
   * agent's permission model — the option_id rides the wire as-is
   * and the agent owns "always" persistence.
   */
  respond: (requestId: string, optionId: string) => Promise<void>
}

export function usePermissions(instanceId?: InstanceId): UsePermissionsApi {
  const { id: activeId } = useActiveInstance()

  const allViews = computed<PermissionView[]>(() => {
    const resolved = instanceId ?? activeId.value

    if (!resolved) {
      return []
    }
    const list = states.get(resolved)

    if (!list || list.length === 0) {
      return []
    }
    const sorted = [...list].sort((a, b) => a.createdAt - b.createdAt)

    return sorted.map((p, i) => buildView(p, i > 0))
  })

  const rowQueue = computed<PermissionView[]>(() => allViews.value.filter((v) => v.call.permissionUi === PermissionUi.Row))

  const modalQueue = computed<PermissionView[]>(() => allViews.value.filter((v) => v.call.permissionUi === PermissionUi.Modal))

  async function respond(requestId: string, optionId: string): Promise<void> {
    const resolved = instanceId ?? activeId.value

    if (!resolved) {
      throw new Error('no active instance')
    }
    const entry = states.get(resolved)?.find((p) => p.requestId === requestId)

    if (!entry) {
      throw new Error(`no pending permission request ${requestId}`)
    }
    await invoke(TauriCommand.PermissionReply, {
      sessionId: entry.sessionId,
      requestId: entry.requestId,
      optionId
    })
    evictPermission(resolved, requestId)
  }

  return {
    rowQueue,
    modalQueue,
    respond
  }
}
