import { computed, ref, type ComputedRef } from 'vue'

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

const states = ref<Record<InstanceId, PendingPermission[]>>({})

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
  const current = states.value[id] ?? []
  const filtered = current.filter((p) => p.requestId !== raw.requestId)

  states.value = { ...states.value, [id]: [...filtered, next] }
  log.trace('permission pending added', {
    instanceId: id,
    requestId: raw.requestId,
    tool: raw.tool,
    size: states.value[id].length
  })
}

export function evictPermission(id: InstanceId, requestId: string): void {
  const current = states.value[id]

  if (!current) {
    return
  }
  const filtered = current.filter((p) => p.requestId !== requestId)

  if (filtered.length === current.length) {
    return
  }
  states.value = { ...states.value, [id]: filtered }
  log.trace('permission pending evicted', {
    instanceId: id,
    requestId,
    size: filtered.length
  })
}

export function resetPermissions(id: InstanceId): void {
  if (!(id in states.value)) {
    return
  }
  const next = { ...states.value }

  delete next[id]
  states.value = next
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
   * pending entry's `options` array. The captain's "remember this"
   * intent is encoded in the option's typed `kind`
   * (`allow_always` / `reject_always`) — the daemon controller reads
   * the kind off the offered set and writes the trust store
   * atomically before signaling the agent. No separate `remember`
   * field on the wire.
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
    const list = states.value[resolved]

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
    const entry = states.value[resolved]?.find((p) => p.requestId === requestId)

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
