import { computed, type ComputedRef, type Ref, ref } from 'vue'

import { invoke, listen, type RemotePairRequestEventPayload, TauriCommand, TauriEvent, type UnlistenFn } from '@ipc'
import { log } from '@lib'

/**
 * Pending pair-request state, surfaced for the desktop overlay to
 * render a confirm modal. The daemon emits one `remote:pair-request`
 * Tauri event per WS upgrade carrying `{ pendingId, code, remoteAddr }`;
 * the captain types or scans the code on the desktop to upgrade the
 * pending WS to authenticated.
 *
 * Single-active-modal model: only the most-recent unresolved request
 * is rendered. Subsequent connections during the modal lifetime queue
 * up; once the captain confirms / rejects the active one, the next
 * pending pair surfaces. Today we keep it simple and just hold the
 * latest one — multi-device queueing lands as a follow-up if real
 * usage shows it's worth the chrome.
 */
export interface RemotePairState {
  pendingId: string
  code: string
  remoteAddr: string
  /** Pre-formatted 4-word phrase split into individual cells. */
  words: string[]
}

const active = ref<RemotePairState | undefined>(undefined)
let unlisten: UnlistenFn | undefined

function toState(payload: RemotePairRequestEventPayload): RemotePairState {
  return {
    pendingId: payload.pendingId,
    code: payload.code,
    remoteAddr: payload.remoteAddr,
    words: payload.code.trim().split(/\s+/u).filter(Boolean)
  }
}

export interface UseRemotePairApi {
  active: Ref<RemotePairState | undefined>
  /** True while a pair request is awaiting captain confirmation. */
  pending: ComputedRef<boolean>
  confirm: (code: string) => Promise<boolean>
  reject: () => Promise<void>
  /** Dismiss the modal without rejecting on the daemon side (rare). */
  dismiss: () => void
}

export function useRemotePair(): UseRemotePairApi {
  return {
    active,
    pending: computed(() => active.value !== undefined),
    confirm: confirmPair,
    reject: rejectPair,
    dismiss: () => {
      active.value = undefined
    }
  }
}

async function confirmPair(code: string): Promise<boolean> {
  const target = active.value

  if (!target) {
    return false
  }

  try {
    const { confirmed } = await invoke(TauriCommand.RemoteConfirmPair, {
      pendingId: target.pendingId,
      code: code.trim()
    })

    if (confirmed) {
      active.value = undefined
    }

    return confirmed
  } catch(err) {
    log.warn('remote: confirm pair failed', { err: String(err) })
    throw err
  }
}

async function rejectPair(): Promise<void> {
  const target = active.value

  if (!target) {
    return
  }

  try {
    await invoke(TauriCommand.RemoteRejectPair, { pendingId: target.pendingId })
  } catch(err) {
    log.warn('remote: reject pair failed', { err: String(err) })
  } finally {
    active.value = undefined
  }
}

/**
 * Mount the `remote:pair-request` listener. Idempotent — repeat calls
 * are a no-op. Returns the unlisten thunk so `App.vue` can release the
 * binding on unmount.
 */
export async function startRemotePairListener(): Promise<UnlistenFn> {
  if (unlisten) {
    return unlisten
  }

  unlisten = await listen(TauriEvent.RemotePairRequest, (event) => {
    log.info('remote: pair request received', {
      pendingId: event.payload.pendingId,
      remoteAddr: event.payload.remoteAddr
    })
    active.value = toState(event.payload)
  })

  return unlisten
}

export function __resetRemotePairForTests(): void {
  active.value = undefined
  unlisten = undefined
}
