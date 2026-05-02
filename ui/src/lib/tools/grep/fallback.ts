import { faMagnifyingGlass } from '@fortawesome/free-solid-svg-icons'

import { pickArgs, shortPath, textBlocks } from '../shared'
import { PermissionUi, PillKind, ToolType } from '@constants/ui'
import type { Formatter } from '@interfaces/ui'

const grepFallback: Formatter = {
  type: ToolType.Grep,
  format(ctx) {
    const { pattern, path, glob, type, outputmode, include } = pickArgs(ctx.args, {
      pattern: 'string',
      path: 'string',
      glob: 'string',
      type: 'string',
      outputmode: 'string',
      include: 'string'
    })
    const where = path ?? '.'
    const bits: string[] = [`in ${shortPath(where)}`]
    const g = glob ?? include

    if (g) {
      bits.push(`glob=${g}`)
    }

    if (type) {
      bits.push(`type=${type}`)
    }

    if (outputmode) {
      bits.push(`mode=${outputmode}`)
    }

    if (ctx.args['-i'] === true) {
      bits.push('-i')
    }

    if (ctx.args['-n'] === true) {
      bits.push('-n')
    }
    const title = pattern ? `grep · ${pattern} · ${bits.join(' ')}` : 'grep'
    const output = textBlocks(ctx.raw.content)

    return {
      id: ctx.raw.id,
      type: ToolType.Grep,
      name: ctx.name,
      state: ctx.state,
      icon: faMagnifyingGlass,
      pill: PillKind.Default,
      permissionUi: PermissionUi.Row,
      title,
      ...(output ? { output } : {}),
      ...(ctx.raw.rawInput ? { rawInput: ctx.raw.rawInput } : {})
    }
  }
}

export default grepFallback
