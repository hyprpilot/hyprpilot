/**
 * Chat-surface UI types — turns, tool chips, the live-row mini list,
 * the session-preview pane.
 */

import type { Phase, ToolState } from '@constants/ui/state'
import type { PlanStatus, ToolKind } from '@constants/ui/chat'

export interface PlanItem {
  status: PlanStatus
  text: string
}

export interface ToolField {
  /// Lowercase short label rendered as the row prefix
  /// ("path" / "pattern" / "tool"). Stays uniform with the existing
  /// hardcoded rows so the spec sheet reads as one grid.
  label: string
  /// Free-form value. Long values wrap; mono-font.
  value: string
}

export interface ToolChipItem {
  /// Short verb word for the chip's text identifier (`Read`,
  /// `Execute`, `Edit`, …).
  label: string
  /// Agent-supplied human-readable title from ACP `tool_call.title`.
  title?: string
  arg?: string
  state: ToolState
  detail?: string
  stat?: string
  kind?: ToolKind
  /// Set when the originating tool call carries a terminal id
  /// (`rawInput.terminal_id`). Drives the inline terminal-card link
  /// from a Bash / Terminal chip.
  terminalId?: string
  /// Markdown-formatted description from the first text content
  /// block — agents (claude-code-acp et al.) routinely emit a
  /// descriptive prose paragraph as the first `ToolCallContent` of
  /// type `text`.
  description?: string
  /// Output payload — terminal stdout / stderr for Bash, file diff
  /// for Write, tool result text for everything else.
  output?: string
  /// Optional structured field rows surfaced under the spec sheet's
  /// hardcoded `command` / `flags` / `detail` lines. Used by the
  /// fallback formatter to surface MCP tool inputs as labelled rows
  /// (`path = …`, `pattern = …`) instead of cramming them all into
  /// a single arg blob. Native-tool formatters can populate this
  /// when their custom rendering wants more than one row of args.
  fields?: ToolField[]
}

/**
 * Past session preview row in the idle-screen mini-list (and the
 * sessions palette).
 */
export interface SessionRowData {
  id: string
  title: string
  cwd: string
  adapter: string
  doing: string
  phase: Phase
}

/**
 * Detailed session preview surfaced in the right-pane of the sessions
 * palette leaf. Carries enough metadata for a quick-read summary
 * (cwd, adapter, last-active timestamp, turn count).
 */
export interface SessionPreview {
  id: string
  title: string
  cwd: string
  adapter: string
  lastActive: string
  turns: number
}
