import { faTerminal } from '@fortawesome/free-solid-svg-icons'

import { pickArgs, textBlocks } from '../shared'
import { PermissionUi, PillKind, ToolType } from '@constants/ui'
import type { Formatter } from '@interfaces/ui'

/**
 * Live terminal session — richer than ad-hoc `bash`. ACP's
 * `ToolCallContent::Terminal` tracks the long-lived terminal id so
 * subsequent stdout / exit notifications attach to the same chip.
 */
const terminalFallback: Formatter = {
  type: ToolType.Terminal,
  format(ctx) {
    const { terminalid, id, command } = pickArgs(ctx.args, {
      terminalid: 'string',
      id: 'string',
      command: 'string'
    })
    const tid = terminalid ?? id
    const output = textBlocks(ctx.raw.content)

    let title: string

    if (command && tid) {
      title = `terminal #${tid} · ${command}`
    } else if (command) {
      title = `terminal · ${command}`
    } else if (tid) {
      title = `terminal #${tid}`
    } else {
      title = 'terminal'
    }

    return {
      id: ctx.raw.id,
      type: ToolType.Terminal,
      name: ctx.name,
      state: ctx.state,
      icon: faTerminal,
      pill: PillKind.Default,
      permissionUi: PermissionUi.Row,
      title,
      ...(output ? { output } : {}),
      ...(ctx.raw.rawInput ? { rawInput: ctx.raw.rawInput } : {})
    }
  }
}

export default terminalFallback
