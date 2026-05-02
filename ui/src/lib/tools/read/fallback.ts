import { faFileLines } from '@fortawesome/free-solid-svg-icons'

import { pickArgs, shortPath, textBlocks } from '../shared'
import { PermissionUi, PillKind, ToolType } from '@constants/ui'
import type { Formatter } from '@interfaces/ui'

const readFallback: Formatter = {
  type: ToolType.Read,
  format(ctx) {
    const { filepath, path, offset, limit } = pickArgs(ctx.args, {
      filepath: 'string',
      path: 'string',
      offset: 'number',
      limit: 'number'
    })
    const p = filepath ?? path
    const trimmed = shortPath(p)

    let title: string

    if (trimmed) {
      if (offset !== undefined && limit !== undefined) {
        title = `read · ${trimmed} (lines ${offset}..${offset + limit})`
      } else if (offset !== undefined) {
        title = `read · ${trimmed} (from ${offset})`
      } else {
        title = `read · ${trimmed}`
      }
    } else {
      title = 'read'
    }

    const output = textBlocks(ctx.raw.content)

    return {
      id: ctx.raw.id,
      type: ToolType.Read,
      name: ctx.name,
      state: ctx.state,
      icon: faFileLines,
      pill: PillKind.Default,
      permissionUi: PermissionUi.Row,
      title,
      ...(output ? { output } : {}),
      ...(ctx.raw.rawInput ? { rawInput: ctx.raw.rawInput } : {})
    }
  }
}

export default readFallback
