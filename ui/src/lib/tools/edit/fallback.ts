import { faPen } from '@fortawesome/free-solid-svg-icons'

import { pickArgs, shortPath, textBlocks } from '../shared'
import { PermissionUi, PillKind, ToolType } from '@constants/ui'
import type { Formatter } from '@interfaces/ui'

const editFallback: Formatter = {
  type: ToolType.Edit,
  format(ctx) {
    const { filepath, path, replaceall } = pickArgs(ctx.args, {
      filepath: 'string',
      path: 'string',
      replaceall: 'boolean'
    })
    const p = filepath ?? path
    const trimmed = shortPath(p)
    const replaceAll = replaceall === true

    let title: string

    if (trimmed) {
      title = replaceAll ? `edit · ${trimmed} (replace all)` : `edit · ${trimmed}`
    } else {
      title = 'edit'
    }

    const output = textBlocks(ctx.raw.content)

    return {
      id: ctx.raw.id,
      type: ToolType.Edit,
      name: ctx.name,
      state: ctx.state,
      icon: faPen,
      pill: PillKind.Default,
      permissionUi: PermissionUi.Row,
      title,
      ...(output ? { output } : {}),
      ...(ctx.raw.rawInput ? { rawInput: ctx.raw.rawInput } : {})
    }
  }
}

export default editFallback
