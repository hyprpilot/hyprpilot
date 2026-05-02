import { faMagnifyingGlassChart } from '@fortawesome/free-solid-svg-icons'

import { pickArgs, textBlocks } from '../shared'
import { PermissionUi, PillKind, ToolType } from '@constants/ui'
import type { Formatter } from '@interfaces/ui'

const toolSearchFallback: Formatter = {
  type: ToolType.ToolSearch,
  format(ctx) {
    const { query, maxresults } = pickArgs(ctx.args, {
      query: 'string',
      maxresults: 'number'
    })
    const max = maxresults
    const suffix = max !== undefined && max > 0 && max !== 5 ? ` (max ${max})` : ''
    const title = query ? `tool search · ${query}${suffix}` : 'tool search'
    const output = textBlocks(ctx.raw.content)

    return {
      id: ctx.raw.id,
      type: ToolType.ToolSearch,
      name: ctx.name,
      state: ctx.state,
      icon: faMagnifyingGlassChart,
      pill: PillKind.Default,
      permissionUi: PermissionUi.Row,
      title,
      ...(output ? { output } : {}),
      ...(ctx.raw.rawInput ? { rawInput: ctx.raw.rawInput } : {})
    }
  }
}

export default toolSearchFallback
