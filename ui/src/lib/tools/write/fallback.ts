import { faPenToSquare } from '@fortawesome/free-solid-svg-icons'

import { pickArgs, shortPath, textBlocks } from '../shared'
import { PermissionUi, PillKind, ToolType } from '@constants/ui'
import type { Formatter } from '@interfaces/ui'

const writeFallback: Formatter = {
  type: ToolType.Write,
  format(ctx) {
    const { filepath, path, content, newstring } = pickArgs(ctx.args, {
      filepath: 'string',
      path: 'string',
      content: 'string',
      newstring: 'string'
    })
    const p = filepath ?? path
    const body = content ?? newstring
    const title = p ? `write · ${shortPath(p)}` : 'write'
    const stat = body && body.length > 0 ? `${body.length} chars` : undefined
    const output = textBlocks(ctx.raw.content)

    return {
      id: ctx.raw.id,
      type: ToolType.Write,
      name: ctx.name,
      state: ctx.state,
      icon: faPenToSquare,
      pill: PillKind.Default,
      permissionUi: PermissionUi.Row,
      title,
      ...(stat ? { stat } : {}),
      ...(output ? { output } : {}),
      ...(ctx.raw.rawInput ? { rawInput: ctx.raw.rawInput } : {})
    }
  }
}

export default writeFallback
