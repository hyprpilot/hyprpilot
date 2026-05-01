import { ToolKind } from '@components'

import { pickString, shortPath } from '../helpers'
import type { ToolFormatter } from '@components'

export const notebookEditFormatter: ToolFormatter = {
  canonical: 'notebook_edit',
  kind: ToolKind.Write,
  format({ args, state }) {
    const path = pickString(args, 'notebookpath', 'filepath')
    const cellId = pickString(args, 'cellid')
    const mode = pickString(args, 'editmode')
    const bits: string[] = []
    if (cellId) {
      bits.push(`cell=${cellId}`)
    }
    if (mode) {
      bits.push(`mode=${mode}`)
    }

    return {
      label: 'Notebook edit',
      arg: shortPath(path),
      detail: bits.length > 0 ? bits.join(' ') : undefined,
      state,
      kind: this.kind
    }
  }
}
