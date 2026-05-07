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
  /** Code shown on the desktop modal as "your code" + QR. */
  desktopCode: string
  /** Code shown on the connecting device — what the desktop must present to confirm. */
  deviceCode: string
  remoteAddr: string
  /** `desktopCode` pre-split into individual cells for rendering. */
  desktopWords: string[]
  /** `deviceCode` pre-split into individual cells for rendering. */
  deviceWords: string[]
}

const active = ref<RemotePairState | undefined>(undefined)
const lastResolution = ref<'confirmed' | 'rejected' | undefined>(undefined)
let unlisten: UnlistenFn | undefined
let unlistenResolved: UnlistenFn | undefined

function toState(payload: RemotePairRequestEventPayload): RemotePairState {
  return {
    pendingId: payload.pendingId,
    desktopCode: payload.desktopCode,
    deviceCode: payload.deviceCode,
    remoteAddr: payload.remoteAddr,
    desktopWords: payload.desktopCode.trim().split(/\s+/u).filter(Boolean),
    deviceWords: payload.deviceCode.trim().split(/\s+/u).filter(Boolean)
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
 * Mount the `remote:pair-request` + `remote:pair-resolved` listeners.
 * Idempotent — repeat calls are a no-op. Returns a thunk that
 * tears down BOTH listeners so `App.vue` can release them on
 * unmount.
 *
 * `pair-resolved` closes the modal whenever the daemon transitions
 * a pending pair out of `pending`, regardless of which side
 * committed — captain confirmed on desktop, device scanned the
 * desktop QR, timeout expired, or the WS dropped. Without this
 * the modal stays open after the device-side confirm path because
 * the desktop never invoked `remote_confirm_pair` itself.
 */
export async function startRemotePairListener(): Promise<UnlistenFn> {
  if (unlisten && unlistenResolved) {
    const teardown = (): void => {
      unlisten?.()
      unlistenResolved?.()
      unlisten = undefined
      unlistenResolved = undefined
    }

    return teardown
  }

  unlisten = await listen(TauriEvent.RemotePairRequest, (event) => {
    log.info('remote: pair request received', {
      pendingId: event.payload.pendingId,
      remoteAddr: event.payload.remoteAddr
    })
    active.value = toState(event.payload)
    lastResolution.value = undefined
  })

  unlistenResolved = await listen(TauriEvent.RemotePairResolved, (event) => {
    const target = active.value

    if (!target || target.pendingId !== event.payload.pendingId) {
      // Resolution for a stale / unknown pending. Common case: the
      // desktop already cleared `active` via its own confirm path
      // (Tauri command), then the WS task emits this for us anyway.
      // Safe to ignore.
      return
    }
    log.info('remote: pair resolved', {
      pendingId: event.payload.pendingId,
      outcome: event.payload.outcome
    })
    lastResolution.value = event.payload.outcome
    active.value = undefined
  })

  return () => {
    unlisten?.()
    unlistenResolved?.()
    unlisten = undefined
    unlistenResolved = undefined
  }
}

export function __resetRemotePairForTests(): void {
  active.value = undefined
  lastResolution.value = undefined
  unlisten = undefined
  unlistenResolved = undefined
}
