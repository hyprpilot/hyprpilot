/**
 * Active-instance store. Single source of truth for "which instance
 * is the user looking at right now" — every per-instance composable
 * (queue / composer / terminals / session-stream / session-info /
 * phase / permissions / tools / transcript) consults this when no
 * explicit id is passed.
 *
 * Two event sources keep the store in sync with the daemon:
 *
 *   - `acp:instances-focused` — direct focus assignment from the
 *     registry; sets the active id verbatim. The daemon owns auto-
 *     focus policy (oldest-survivor on shutdown, first-spawn on
 *     empty), so this is the authoritative signal.
 *   - `acp:instance-state` — lifecycle transitions feed the toast
 *     tone: when focus moves involuntarily, the prior instance's
 *     last `Ended` (clean shutdown) vs `Error` (crash) state picks
 *     ok-vs-warn for the surfacing toast. The store carries no
 *     focus mutation off this event — it's diagnostic only.
 *
 * `setIfUnset` survives as the bootstrap path: the very first
 * `Running` state in `useSessionStream` calls it before the daemon
 * has had a chance to emit a focus event. Once focus events start
 * flowing they take precedence.
 */
import { computed, ref, type ComputedRef, type Ref } from 'vue'

import { peekSessionInfo } from '../instance/use-session-info'
import { pushToast } from '../ui-state/use-toasts'
import { ToastTone } from '@components'
import { TauriEvent, InstanceState } from '@ipc'
import { listen, type UnlistenFn } from '@ipc/bridge'

export type InstanceId = string

interface InstanceMeta {
  agentId?: string
  /** Last lifecycle state observed for this instance. */
  lastState?: InstanceState
}

const activeId = ref<InstanceId>()
const instanceIds = ref<InstanceId[]>([])
const knownInstances = new Map<InstanceId, InstanceMeta>()
let started = false
let unlisteners: UnlistenFn[] = []

/**
 * Update the recorded lifecycle state for an instance. Called by the
 * session-stream demuxer on every `acp:instance-state` event so the
 * focus-shift toast can pick a tone derived from the prior instance's
 * exit reason.
 */
export function recordInstanceState(id: InstanceId, agentId: string | undefined, state: InstanceState): void {
  let meta = knownInstances.get(id)

  if (!meta) {
    meta = {}
    knownInstances.set(id, meta)
  }

  if (agentId !== undefined) {
    meta.agentId = agentId
  }
  meta.lastState = state
}

function applyFocus(next: InstanceId | undefined, reason: 'event' | 'manual'): void {
  const prev = activeId.value

  activeId.value = next

  if (reason !== 'event') {
    return
  }

  if (prev && prev !== next) {
    const meta = knownInstances.get(prev)
    const tone = meta?.lastState === InstanceState.Error ? ToastTone.Warn : ToastTone.Ok
    const prevLabel = labelFor(prev, meta)

    if (next) {
      const nextLabel = labelFor(next, knownInstances.get(next))

      pushToast(tone, `${prevLabel} exited — switched to ${nextLabel}`)
    } else {
      pushToast(tone, `${prevLabel} exited`)
    }
  }
}

/**
 * Compose a captain-readable instance label from the per-instance
 * sessionInfo slot. Prefers profile id (the captain's named bundle);
 * falls back through model → agent → first 8 of the UUID. The
 * fallback chain matches what's shown in the header chips so the
 * toast reads in the same vocabulary as the chrome.
 */
function labelFor(id: InstanceId, meta: InstanceMeta | undefined): string {
  const info = peekSessionInfo(id)
  const parts: string[] = []

  if (info?.profileId) {
    parts.push(info.profileId)
  }

  if (info?.model) {
    parts.push(info.model)
  } else if (info?.agent) {
    parts.push(info.agent)
  } else if (meta?.agentId) {
    parts.push(meta.agentId)
  }

  if (parts.length === 0) {
    return id.slice(0, 8)
  }

  return parts.join(' · ')
}

/**
 * Subscribe the active-instance store to the daemon's focus + state
 * events. Idempotent — second call returns the existing teardown fn.
 * Pair with `stopActiveInstance` for tests / unmount.
 */
export async function startActiveInstance(): Promise<() => void> {
  if (started) {
    return stopActiveInstance
  }
  started = true
  unlisteners.push(
    await listen(TauriEvent.AcpInstancesFocused, (e) => {
      applyFocus(e.payload.instanceId, 'event')
    }),
    await listen(TauriEvent.AcpInstancesChanged, (e) => {
      const ids = new Set(e.payload.instanceIds)

      for (const known of [...knownInstances.keys()]) {
        if (!ids.has(known)) {
          knownInstances.delete(known)
        }
      }

      for (const id of e.payload.instanceIds) {
        if (!knownInstances.has(id)) {
          knownInstances.set(id, {})
        }
      }
      instanceIds.value = [...e.payload.instanceIds]

      // The daemon publishes `instances/focused` separately; trust it
      // for the active id rather than guessing from the membership
      // delta. Only act here when the current active id is no longer
      // a member and no focused-id is supplied — defensive against
      // out-of-order delivery.
      if (activeId.value && !ids.has(activeId.value) && e.payload.focusedId === undefined) {
        applyFocus(undefined, 'event')
      }
    })
  )

  return stopActiveInstance
}

export function stopActiveInstance(): void {
  for (const u of unlisteners) {
    u()
  }
  unlisteners = []
  started = false
}

/**
 * Test-only reset. Drops the focus pointer + the known-instance map +
 * tears down any live event listeners.
 */
export function __resetActiveInstanceForTests(): void {
  stopActiveInstance()
  activeId.value = undefined
  instanceIds.value = []
  knownInstances.clear()
}

export function useActiveInstance(): {
  id: Ref<InstanceId | undefined>
  ids: Ref<InstanceId[]>
  count: ComputedRef<number>
  set: (next: InstanceId) => void
  setIfUnset: (next: InstanceId) => void
  clear: () => void
} {
  function set(next: InstanceId): void {
    applyFocus(next, 'manual')
  }

  function setIfUnset(next: InstanceId): void {
    if (!activeId.value) {
      applyFocus(next, 'manual')
    }
  }

  function clear(): void {
    applyFocus(undefined, 'manual')
  }

  return {
    id: activeId,
    ids: instanceIds,
    count: computed(() => instanceIds.value.length),
    set,
    setIfUnset,
    clear
  }
}
