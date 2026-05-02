import { faPen } from '@fortawesome/free-solid-svg-icons'

import { pickArgs, shortPath, textBlocks } from '../shared'
import { PermissionUi, PillKind, ToolType } from '@constants/ui'
import type { Formatter } from '@interfaces/ui'

const multiEditFallback: Formatter = {
  type: ToolType.MultiEdit,
  format(ctx) {
    const { filepath, path, edits } = pickArgs(ctx.args, {
      filepath: 'string',
      path: 'string',
      edits: 'list'
    })
    const p = filepath ?? path
    const count = edits?.length ?? 0
    const title = p ? `edit · ${shortPath(p)}` : 'edit'
    const stat = count > 0 ? `${count} ${count === 1 ? 'edit' : 'edits'}` : undefined
    const output = textBlocks(ctx.raw.content)

    return {
      id: ctx.raw.id,
      type: ToolType.MultiEdit,
      name: ctx.name,
      state: ctx.state,
      icon: faPen,
      pill: PillKind.Default,
      permissionUi: PermissionUi.Row,
      title,
      ...(stat ? { stat } : {}),
      ...(output ? { output } : {}),
      ...(ctx.raw.rawInput ? { rawInput: ctx.raw.rawInput } : {})
    }
  }
}

export default multiEditFallback
