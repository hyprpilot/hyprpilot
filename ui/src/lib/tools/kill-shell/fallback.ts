import { faSkull } from '@fortawesome/free-solid-svg-icons'

import { pickArgs } from '../shared'
import { PermissionUi, PillKind, ToolType } from '@constants/ui'
import type { Formatter } from '@interfaces/ui'

const killShellFallback: Formatter = {
  type: ToolType.KillShell,
  format(ctx) {
    const { shellid, bashid } = pickArgs(ctx.args, { shellid: 'string', bashid: 'string' })
    const id = shellid ?? bashid
    const title = id ? `kill shell #${id}` : 'kill shell'

    return {
      id: ctx.raw.id,
      type: ToolType.KillShell,
      name: ctx.name,
      state: ctx.state,
      icon: faSkull,
      pill: PillKind.Default,
      permissionUi: PermissionUi.Row,
      title,
      ...(ctx.raw.rawInput ? { rawInput: ctx.raw.rawInput } : {})
    }
  }
}

export default killShellFallback
