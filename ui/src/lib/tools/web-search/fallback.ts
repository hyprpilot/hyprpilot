import { faMagnifyingGlassPlus } from '@fortawesome/free-solid-svg-icons'

import { pickArgs, textBlocks } from '../shared'
import { PermissionUi, PillKind, ToolType } from '@constants/ui'
import type { Formatter } from '@interfaces/ui'

const webSearchFallback: Formatter = {
  type: ToolType.WebSearch,
  format(ctx) {
    const { query, alloweddomains, blockeddomains } = pickArgs(ctx.args, {
      query: 'string',
      alloweddomains: 'stringList',
      blockeddomains: 'stringList'
    })
    const bits: string[] = []

    if (alloweddomains && alloweddomains.length > 0) {
      bits.push(`allowed: ${alloweddomains.join(', ')}`)
    }

    if (blockeddomains && blockeddomains.length > 0) {
      bits.push(`blocked: ${blockeddomains.join(', ')}`)
    }
    const suffix = bits.length > 0 ? ` (${bits.join(' · ')})` : ''
    const title = query ? `search · ${query}${suffix}` : 'search'
    const output = textBlocks(ctx.raw.content)

    return {
      id: ctx.raw.id,
      type: ToolType.WebSearch,
      name: ctx.name,
      state: ctx.state,
      icon: faMagnifyingGlassPlus,
      pill: PillKind.Default,
      permissionUi: PermissionUi.Row,
      title,
      ...(output ? { output } : {}),
      ...(ctx.raw.rawInput ? { rawInput: ctx.raw.rawInput } : {})
    }
  }
}

export default webSearchFallback
