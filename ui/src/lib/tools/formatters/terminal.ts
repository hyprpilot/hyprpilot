import { ToolKind } from '@components'

import { pickString } from '../helpers'
import type { ToolFormatter } from '@components'

export const terminalFormatter: ToolFormatter = {
  canonical: 'terminal',
  kind: ToolKind.Terminal,
  format({ args, state }) {
    const terminalId = pickString(args, 'terminalid', 'id')
    const command = pickString(args, 'command')

    return {
      label: 'Terminal',
      arg: terminalId || command,
      detail: terminalId && command ? command : undefined,
      state,
      kind: this.kind,
      terminalId: terminalId || undefined
    }
  }
}
