import { faPuzzlePiece } from '@fortawesome/free-solid-svg-icons'

import { pickArgs, textBlocks } from '../shared'
import { PermissionUi, PillKind, ToolType } from '@constants/ui'
import type { Formatter } from '@interfaces/ui'

/**
 * Skill — claude-code-acp's tool for invoking a registered skill
 * bundle. Wire shape carries the slug under `skill`. Without a
 * dedicated formatter the slug ends up as a JSON dump in the chip;
 * surface it as the title suffix instead.
 */
const skillFallback: Formatter = {
  type: ToolType.Skill,
  format(ctx) {
    const { skill, description } = pickArgs(ctx.args, {
      skill: 'string',
      description: 'string'
    })
    const title = skill ? `skill · ${skill}` : 'skill'
    const output = textBlocks(ctx.raw.content)

    return {
      id: ctx.raw.id,
      type: ToolType.Skill,
      name: ctx.name,
      state: ctx.state,
      icon: faPuzzlePiece,
      pill: PillKind.Default,
      permissionUi: PermissionUi.Row,
      title,
      ...(description ? { description } : {}),
      ...(output ? { output } : {}),
      ...(ctx.raw.rawInput ? { rawInput: ctx.raw.rawInput } : {})
    }
  }
}

export default skillFallback
