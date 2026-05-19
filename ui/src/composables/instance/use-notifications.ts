/**
 * Daemon-side "needs attention" tracker — Vue mirror.
 *
 * Single-source-of-truth shape: the daemon ships a full snapshot on
 * `acp:notifications-changed` (idempotent on lossy broadcast — every
 * event re-asserts the canonical list). The composable hydrates from
 * the boot snapshot, then re-renders off subsequent events. No
 * client-side delta tracking — the daemon is authoritative.
 *
 * `dismiss(instanceId)` and `dismissAll()` invoke the matching Tauri
 * commands; the daemon broadcasts the cleared state back so the
 * mirror updates via the same event path as any other transition.
 * The composable does NOT optimistically prune — the round-trip is
 * cheap and the wire is the truth.
 */

import { ref, computed, type Ref } from 'vue'

import { invoke, listen, TauriCommand, TauriEvent, type NotificationEntry, type UnlistenFn } from '@ipc'
import { log } from '@lib'

const items = ref<NotificationEntry[]>([])
let unlisten: UnlistenFn | undefined
let subscribed = false

export interface UseNotificationsApi {
  items: Ref<NotificationEntry[]>
  count: Ref<number>
  /** Manually clear a single entry — the daemon's normal resolution
   *  paths (focus, permission resolved, prompt sent) cover the common
   *  cases. */
  dismiss: (instanceId: string) => Promise<void>
  /** Dismiss every pending entry at once. Captain hit the "clear all"
   *  affordance. */
  dismissAll: () => Promise<void>
}

/** Boot-snapshot seed. Called once from `applyBootSnapshot`. */
export function applyBootNotifications(snap: { items: NotificationEntry[] } | undefined): void {
  if (!snap) {
    return
  }
  items.value = snap.items
}

async function ensureSubscribed(): Promise<void> {
  if (subscribed) {
    return
  }
  subscribed = true

  try {
    unlisten = await listen(TauriEvent.AcpNotificationsChanged, (e) => {
      items.value = e.payload.items
    })
  } catch(err) {
    subscribed = false
    log.warn('useNotifications: subscribe failed', undefined, err)
  }
}

/** Test-only reset — drops state + listener so each vitest case starts clean. */
export function __resetNotificationsForTests(): void {
  items.value = []

  if (unlisten) {
    unlisten()
    unlisten = undefined
  }
  subscribed = false
}

export function useNotifications(): UseNotificationsApi {
  void ensureSubscribed()

  return {
    items,
    count: computed(() => items.value.length),
    dismiss: async(instanceId: string) => {
      try {
        await invoke(TauriCommand.NotificationsClear, { instanceId })
      } catch(err) {
        log.warn('useNotifications.dismiss failed', { instanceId }, err)
      }
    },
    dismissAll: async() => {
      try {
        await invoke(TauriCommand.NotificationsClearAll)
      } catch(err) {
        log.warn('useNotifications.dismissAll failed', undefined, err)
      }
    }
  }
}
