import { ToolKind } from '@components'

import { pickString, shortPath } from '../helpers'
import type { ToolFormatter } from '@components'

export const grepFormatter: ToolFormatter = {
  canonical: 'grep',
  kind: ToolKind.Search,
  format({ args, state }) {
    const pattern = pickString(args, 'pattern')
    const path = pickString(args, 'path') || '.'
    const glob = pickString(args, 'glob', 'include')
    const type = pickString(args, 'type')
    const mode = pickString(args, 'outputmode')
    const bits: string[] = [`in ${shortPath(path)}`]
    if (glob) {
      bits.push(`glob=${glob}`)
    }
    if (type) {
      bits.push(`type=${type}`)
    }
    if (mode) {
      bits.push(`mode=${mode}`)
    }
    if (args['-i']) {
      bits.push('-i')
    }
    if (args['-n']) {
      bits.push('-n')
    }

    return {
      label: 'Grep',
      arg: pattern,
      detail: bits.join(' '),
      state,
      kind: this.kind
    }
  }
}
