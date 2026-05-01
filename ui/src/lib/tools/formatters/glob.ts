import { ToolKind } from '@components'

import { pickString, shortPath } from '../helpers'
import type { ToolFormatter } from '@components'

export const globFormatter: ToolFormatter = {
  canonical: 'glob',
  kind: ToolKind.Search,
  format({ args, state }) {
    const pattern = pickString(args, 'pattern')
    const path = pickString(args, 'path')

    return {
      label: 'Glob',
      arg: pattern,
      detail: path ? `in ${shortPath(path)}` : undefined,
      state,
      kind: this.kind
    }
  }
}
