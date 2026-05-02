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
  /// ACP-supplied option set ("Allow" / "Deny" / "Always allow"
  /// / "Always deny"). Each carries a typed `optionId`.
  options: PermissionOptionView[]
  /// Set when more than one prompt is pending and this one is
  /// behind the head.
  queued?: boolean
}
