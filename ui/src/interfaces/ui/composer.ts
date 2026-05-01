/**
 * Composer-surface UI types — pending permission rows, queued
 * messages, attachment / resource pills.
 */

import type { ComposerPillKind } from '@constants/ui/composer'

export interface PermissionPrompt {
  id: string
  tool: string
  kind: string
  args: string
  /// Raw `tool_call.rawInput` JSON (pass-through). Special-case
  /// renderers (the ExitPlanMode plan modal) read structured fields
  /// here — `plan` for ExitPlanMode, etc.
  rawInput?: Record<string, unknown>
  queued?: boolean
}

export interface QueuedMessage {
  id: string
  text: string
}

/**
 * Attachment / resource chip in the composer row. `data` is wire
 * payload — a file path / URL for resources, base64 image bytes for
 * attachments. `mimeType` is set on attachments so the submit path
 * can map to the right ACP `ContentBlock` variant.
 */
export interface ComposerPill {
  kind: ComposerPillKind
  id: string
  label: string
  data: string
  mimeType?: string
  /// Original filename for image attachments — used to seed the
  /// wire `path` so `mime_guess` can fall back on the extension
  /// when the explicit MIME is unavailable. Optional; clipboard
  /// pastes have no name and synthesize from the MIME instead.
  fileName?: string
}
