import { ToolKind } from '@components'

import { pickString, shortPath } from '../helpers'
import type { ToolFormatter } from '@components'

export const writeFormatter: ToolFormatter = {
  canonical: 'write',
  kind: ToolKind.Write,
  format({ args, state }) {
    const path = pickString(args, 'filepath', 'path')
    const content = pickString(args, 'content', 'newstring')
    const stat = content.length > 0 ? `${content.length} chars` : undefined

    return {
      label: 'Write',
      arg: shortPath(path),
      stat,
      state,
      kind: this.kind
    }
  }
}
