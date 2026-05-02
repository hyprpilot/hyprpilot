/**
 * Chat-surface UI types — turns, plan items, the live-row mini list,
 * the session-preview pane. Tool-call view types live in `tools.ts`.
 */

import type { PlanStatus } from '@constants/ui/chat'
import type { Phase } from '@constants/ui/state'

export interface PlanItem {
  status: PlanStatus
  text: string
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
