import { faUserGear } from '@fortawesome/free-solid-svg-icons'

import { pickArgs, textBlocks } from '../shared'
import { PermissionUi, PillKind, ToolType } from '@constants/ui'
import type { Formatter } from '@interfaces/ui'

const taskFallback: Formatter = {
  type: ToolType.Task,
  format(ctx) {
    const { subagenttype, description, prompt } = pickArgs(ctx.args, {
      subagenttype: 'string',
      description: 'string',
      prompt: 'string'
    })
    const subagent = subagenttype ?? 'agent'
    const title = description ? `task · ${subagent} — ${description}` : `task · ${subagent}`
    const body = prompt ? (prompt.length > 200 ? `${prompt.slice(0, 199)}…` : prompt) : undefined
    const output = textBlocks(ctx.raw.content)

    return {
      id: ctx.raw.id,
      type: ToolType.Task,
      name: ctx.name,
      state: ctx.state,
      icon: faUserGear,
      pill: PillKind.Default,
      permissionUi: PermissionUi.Row,
      title,
      ...(body ? { description: body } : {}),
      ...(output ? { output } : {}),
      ...(ctx.raw.rawInput ? { rawInput: ctx.raw.rawInput } : {})
    }
  }
}

export default taskFallback
