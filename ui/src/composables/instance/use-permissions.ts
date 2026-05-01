import { computed, ref, type ComputedRef } from 'vue'

import { type PermissionPrompt } from '@components'
import { invoke, TauriCommand, type PermissionOptionView } from '@ipc'
import { log } from '@lib'

import { nextSeq } from './sequence'
import { useActiveInstance, type InstanceId } from '../chrome/use-active-instance'

/**
 * Stored shape — `queued` is derived at read time in `pending`
 * (oldest-by-createdAt is active, everything else queued), not
 * snapshotted at insert. Snapshotting desyncs after eviction: the
 * remaining rows keep `queued: true` and the stack's activeId
 * lookup (`prompts.find((p) => !p.queued)`) goes undefined — action
 * buttons disappear.
 */
export interface PendingPermission extends Omit<PermissionPrompt, 'queued'> {
  instanceId: InstanceId
  requestId: string
  sessionId: string
  createdAt: number
  options: PermissionOptionView[]
  /// Pass-through of `tool_call.rawInput`. Consumers checking the
  /// permission shape (e.g. the plan-file modal) read structured
  /// fields directly here.
  rawInput?: Record<string, unknown>
  /// Joined text from the tool-call's `content[]` blocks. Modal
  /// reads as a fallback markdown body when no `rawInput` field
  /// matches the body-shape detector.
  contentText?: string
}

export interface PermissionRequestRaw {
  requestId: string
  tool: string
  kind?: string
  args?: string
  rawInput?: Record<string, unknown>
  contentText?: string
  options: PermissionOptionView[]
}

export enum PermissionDecision {
  Allow = 'allow',
  Deny = 'deny'
}

/**
 * Per-instance pending list. Stored as a `ref` over a plain
 * `Record<InstanceId, PendingPermission[]>` rather than
 * `reactive(Map)` because every mutation REPLACES the slot's array
 * (and the outer record) so Vue's reactive tracking fires on the
 * wrapping `ref` regardless of how clever the consuming computed
 * gets. The previous `reactive(new Map())` shape would silently miss
 * updates when a second permission landed before the first was
 * resolved — the UI showed the head and never re-rendered when the
 * map's `size` changed inside a deeply-nested computed.
 */
const states = ref<Record<InstanceId, PendingPermission[]>>({})

/**
 * Accumulates a pending permission prompt for the given instance.
 * Keyed by `requestId`; re-pushing the same id replaces the entry.
 */
export function pushPermissionRequest(id: InstanceId, sessionId: string, raw: PermissionRequestRaw): void {
  const seq = nextSeq(id)
  const next: PendingPermission = {
    instanceId: id,
    requestId: raw.requestId,
    sessionId,
    id: raw.requestId,
    tool: raw.tool,
    kind: raw.kind ?? 'acp',
    args: raw.args ?? '',
    rawInput: raw.rawInput,
    contentText: raw.contentText,
    createdAt: seq,
    options: raw.options
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
  log.trace('permission pending evicted', { instanceId: id, requestId, size: filtered.length })
}

export function resetPermissions(id: InstanceId): void {
  if (!(id in states.value)) {
    return
  }
  const next = { ...states.value }
  delete next[id]
  states.value = next
}

export interface PendingPermissionView extends PendingPermission {
  queued: boolean
}

export function usePermissions(instanceId?: InstanceId): {
  pending: ComputedRef<PendingPermissionView[]>
  /**
   * Allow a pending permission. `remember=true` writes a runtime
   * trust-store entry for `(instance, tool)` so subsequent calls of
   * the same tool short-circuit at decide() lane 1 — that's the UI's
   * "always allow" path. `remember=false` (default) is "once".
   */
  allow: (requestId: string, remember?: boolean) => Promise<void>
  deny: (requestId: string, remember?: boolean) => Promise<void>
} {
  const { id: activeId } = useActiveInstance()

  const pending = computed<PendingPermissionView[]>(() => {
    const resolved = instanceId ?? activeId.value
    if (!resolved) {
      return []
    }
    const list = states.value[resolved]
    if (!list || list.length === 0) {
      return []
    }
    const sorted = [...list].sort((a, b) => a.createdAt - b.createdAt)

    return sorted.map((p, i) => ({ ...p, queued: i > 0 }))
  })

  async function respond(requestId: string, decision: PermissionDecision, remember: boolean): Promise<void> {
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
      optionId: decision,
      remember: remember ? decision : undefined,
      instanceId: entry.instanceId,
      tool: entry.tool
    })
    evictPermission(resolved, requestId)
  }

  return {
    pending,
    allow: (requestId, remember = false) => respond(requestId, PermissionDecision.Allow, remember),
    deny: (requestId, remember = false) => respond(requestId, PermissionDecision.Deny, remember)
  }
}
