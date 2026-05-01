import { ToolKind } from '@components'

import { pickString } from '../helpers'
import type { ToolFormatter } from '@components'

export const bashFormatter: ToolFormatter = {
  canonical: 'bash',
  kind: ToolKind.Bash,
  format({ args, state }) {
    const command = pickString(args, 'command')
    const description = pickString(args, 'description')
    const background = Boolean(args.runinbackground)
    const terminalId = pickString(args, 'terminalid', 'id')
    let detail: string | undefined
    if (description && background) {
      detail = `${description} (background)`
    } else if (description) {
      detail = description
    } else if (background) {
      detail = 'background'
    }

    return {
      label: 'Bash',
      arg: command,
      detail,
      state,
      kind: this.kind,
      terminalId: terminalId || undefined
    }
  }
}

export const bashOutputFormatter: ToolFormatter = {
  canonical: 'bash_output',
  kind: ToolKind.Bash,
  format({ args, state }) {
    const shellId = pickString(args, 'bashid', 'shellid')
    const filter = pickString(args, 'filter')

    return {
      label: 'Bash output',
      arg: shellId,
      detail: filter ? `filter ${filter}` : undefined,
      state,
      kind: this.kind
    }
  }
}

export const killShellFormatter: ToolFormatter = {
  canonical: 'kill_shell',
  kind: ToolKind.Bash,
  format({ args, state }) {
    const shellId = pickString(args, 'shellid', 'bashid')

    return {
      label: 'Kill shell',
      arg: shellId,
      state,
      kind: this.kind
    }
  }
}
