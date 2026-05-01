import { ToolKind } from '@components'

import { pickList, pickString, shortPath } from '../helpers'
import type { ToolFormatter } from '@components'

export const editFormatter: ToolFormatter = {
  canonical: 'edit',
  aliases: ['patch'],
  kind: ToolKind.Write,
  format({ args, state }) {
    const path = pickString(args, 'filepath', 'path')
    const replaceAll = Boolean(args.replaceall)
    const detail = replaceAll ? 'replace all' : undefined

    return {
      label: 'Edit',
      arg: shortPath(path),
      detail,
      state,
      kind: this.kind
    }
  }
}

export const multiEditFormatter: ToolFormatter = {
  canonical: 'multi_edit',
  kind: ToolKind.Write,
  format({ args, state }) {
    const path = pickString(args, 'filepath', 'path')
    const edits = pickList(args, 'edits')
    const stat = edits ? `${edits.length} edits` : undefined

    return {
      label: 'Multi edit',
      arg: shortPath(path),
      stat,
      state,
      kind: this.kind
    }
  }
}
