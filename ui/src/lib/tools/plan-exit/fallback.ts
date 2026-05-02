import { faClipboardList } from '@fortawesome/free-solid-svg-icons'

import { pickArgs } from '../shared'
import { PermissionUi, PillKind, ToolType } from '@constants/ui'
import type { Formatter } from '@interfaces/ui'

/**
 * Plan-exit (`ExitPlanMode`) — the canonical `permissionUi: Modal`
 * driver. Args carry the plan body as markdown; the modal renders
 * it via `<MarkdownBody>` so the captain reviews + accepts/rejects
 * the plan before the agent leaves plan mode.
 */
const planExitFallback: Formatter = {
  type: ToolType.PlanExit,
  format(ctx) {
    const { plan } = pickArgs(ctx.args, { plan: 'string' })
    const title = 'plan ready for review'

    return {
      id: ctx.raw.id,
      type: ToolType.PlanExit,
      name: ctx.name,
      state: ctx.state,
      icon: faClipboardList,
      pill: PillKind.Default,
      permissionUi: PermissionUi.Modal,
      title,
      ...(plan ? { description: plan } : {}),
      ...(ctx.raw.rawInput ? { rawInput: ctx.raw.rawInput } : {})
    }
  }
}

export default planExitFallback
