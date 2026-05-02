import { faBrain } from '@fortawesome/free-solid-svg-icons'

import { pickArgs, textBlocks } from '../shared'
import { PermissionUi, PillKind, ToolType } from '@constants/ui'
import type { Formatter } from '@interfaces/ui'

/**
 * Think — agent's internal reasoning surfaced as a tool call by some
 * adapters (claude-code-acp, codex's `GuardianAssessment`). The chat
 * surface routes thought calls to a separate thoughts list (rendered
 * as a thinking-card) rather than the tool-pill row; this formatter
 * still produces a complete `ToolCallView` so the data is uniform.
 */
const thinkFallback: Formatter = {
  type: ToolType.Think,
  format(ctx) {
    const { thought,
      text,
      description: descArg } = pickArgs(ctx.args, {
      thought: 'string',
      text: 'string',
      description: 'string'
    })
    const body = thought ?? text ?? descArg
    const blocks = textBlocks(ctx.raw.content)
    const description = body ?? blocks
    const title = ctx.raw.title?.trim() || 'thinking'

    return {
      id: ctx.raw.id,
      type: ToolType.Think,
      name: ctx.name,
      state: ctx.state,
      icon: faBrain,
      pill: PillKind.Default,
      permissionUi: PermissionUi.Row,
      title,
      ...(description ? { description } : {}),
      ...(ctx.raw.rawInput ? { rawInput: ctx.raw.rawInput } : {})
    }
  }
}

export default thinkFallback
