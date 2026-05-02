import { faListCheck } from '@fortawesome/free-solid-svg-icons'

import { pickArgs, textBlocks } from '../shared'
import { PermissionUi, PillKind, ToolType } from '@constants/ui'
import type { Formatter, ToolField } from '@interfaces/ui'

interface TodoEntry {
  content?: string
  status?: string
  activeForm?: string
}

const todoFallback: Formatter = {
  type: ToolType.Todo,
  format(ctx) {
    const { todos } = pickArgs(ctx.args, { todos: 'list' })
    const list = (todos ?? []).filter((t): t is TodoEntry => Boolean(t) && typeof t === 'object')
    const count = list.length

    const counts: Record<string, number> = {}

    for (const entry of list) {
      const status = entry.status

      if (typeof status === 'string') {
        counts[status] = (counts[status] ?? 0) + 1
      }
    }
    const breakdown = Object.entries(counts)
      .map(([k, v]) => `${k}:${v}`)
      .join(' ')

    const title = count > 0 ? `todos · ${count} ${count === 1 ? 'item' : 'items'}` : 'todos'
    const stat = breakdown.length > 0 ? breakdown : undefined

    const fields: ToolField[] = list.flatMap((entry) => {
      const text = entry.content ?? entry.activeForm

      if (!text) {
        return []
      }

      return [{ label: entry.status ?? 'todo', value: text }]
    })

    const output = textBlocks(ctx.raw.content)

    return {
      id: ctx.raw.id,
      type: ToolType.Todo,
      name: ctx.name,
      state: ctx.state,
      icon: faListCheck,
      pill: PillKind.Default,
      permissionUi: PermissionUi.Row,
      title,
      ...(stat ? { stat } : {}),
      ...(fields.length > 0 ? { fields } : {}),
      ...(output ? { output } : {}),
      ...(ctx.raw.rawInput ? { rawInput: ctx.raw.rawInput } : {})
    }
  }
}

export default todoFallback
