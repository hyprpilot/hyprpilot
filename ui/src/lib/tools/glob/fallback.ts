import { faStarOfLife } from '@fortawesome/free-solid-svg-icons'

import { pickArgs, shortPath, textBlocks } from '../shared'
import { PermissionUi, PillKind, ToolType } from '@constants/ui'
import type { Formatter } from '@interfaces/ui'

const globFallback: Formatter = {
  type: ToolType.Glob,
  format(ctx) {
    const { pattern, path } = pickArgs(ctx.args, { pattern: 'string', path: 'string' })
    const trimmed = shortPath(path)
    let title: string

    if (pattern && trimmed) {
      title = `glob · ${pattern} in ${trimmed}`
    } else if (pattern) {
      title = `glob · ${pattern}`
    } else {
      title = 'glob'
    }
    const output = textBlocks(ctx.raw.content)

    return {
      id: ctx.raw.id,
      type: ToolType.Glob,
      name: ctx.name,
      state: ctx.state,
      icon: faStarOfLife,
      pill: PillKind.Default,
      permissionUi: PermissionUi.Row,
      title,
      ...(output ? { output } : {}),
      ...(ctx.raw.rawInput ? { rawInput: ctx.raw.rawInput } : {})
    }
  }
}

export default globFallback
