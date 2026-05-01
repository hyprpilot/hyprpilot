import { ToolKind } from '@components'

import { pickNumber, pickString, shortPath } from '../helpers'
import type { ToolFormatter } from '@components'

export const readFormatter: ToolFormatter = {
  canonical: 'read',
  kind: ToolKind.Read,
  format({ args, state }) {
    const path = pickString(args, 'filepath', 'path')
    const offset = pickNumber(args, 'offset')
    const limit = pickNumber(args, 'limit')
    let detail: string | undefined
    if (offset !== undefined && limit !== undefined) {
      detail = `lines ${offset}..${offset + limit}`
    } else if (offset !== undefined) {
      detail = `from line ${offset}`
    }

    return {
      label: 'Read',
      arg: shortPath(path),
      detail,
      state,
      kind: this.kind
    }
  }
}
