/**
 * Daemon-side "needs attention" tracker. The daemon raises an entry
 * per instance when one of three things happens (turn ended, permission
 * requested, instance entered Error) on a non-focused instance; the
 * captain dismisses by focusing, answering a permission, sending a
 * prompt, or hitting "dismiss all".
 *
 * Wire shape mirrors `src-tauri/src/adapters/notifications.rs` —
 * `serde(rename_all = "snake_case")` on `NotificationReason` projects
 * the enum variants onto the lowercase strings below.
 */

export enum NotificationReason {
  TurnEnded = 'turn_ended',
  PermissionRequested = 'permission_requested',
  InstanceError = 'instance_error'
}

export interface NotificationEntry {
  instanceId: string
  /** Sorted ascending — the daemon ships a deterministic order. */
  reasons: NotificationReason[]
  /** Epoch ms when the entry was first raised. Sticky across re-raises. */
  since: number
}

export interface NotificationsSnapshot {
  items: NotificationEntry[]
}

export interface NotificationsClearArgs {
  instanceId: string
}

export interface NotificationsGetArgs {
  instanceId: string
}

/** `entry: null` when the instance has nothing pending. */
export interface NotificationsGetResult {
  entry: NotificationEntry | null
}

export interface NotificationsChangedEventPayload {
  items: NotificationEntry[]
}
