/**
 * Permission-flow types. `PermissionView` wraps a formatted
 * `ToolCallView` with the wire metadata `ToolCallView` doesn't carry
 * — request id, options, instance/session ids for trust-store
 * keying. `usePermissions` produces these and splits row vs modal
 * queues by `view.call.permissionUi`.
 */

import type { ToolCallView } from './tools'
import type { PermissionOptionView } from '@interfaces/wire'

export interface PermissionRequest {
  /// `permission_reply { request_id }` target.
  requestId: string
  /// Trust-store keying.
  instanceId: string
  sessionId: string
  /// Raw wire tool name — trust-store key + glob-match key.
  toolName: string
}

export interface PermissionView {
  request: PermissionRequest
  /// Formatted view drives ALL chrome (icon, title, fields, etc.).
  call: ToolCallView
  /// Options pre-sorted by the daemon: allow_always, allow_once,
  /// reject_once, then rest.
  options: PermissionOptionView[]
  /// Default-highlight option id — primary (solid) button + the
  /// `Enter`-commit target. Mirror of `allowOptionId`. `undefined`
  /// when the agent didn't offer an allow_once option.
  defaultOptionId?: string
  /// Allow keybind (Ctrl+G) target. `undefined` → keybind shows a
  /// toast instead of firing.
  allowOptionId?: string
  /// Deny keybind (Ctrl+R) target. Same toast-when-undefined rule.
  rejectOptionId?: string
  /// Set when more than one prompt is pending and this one is
  /// behind the head.
  queued?: boolean
}
