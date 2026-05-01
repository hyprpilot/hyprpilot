import { ToolKind } from '@components'

import { pickList } from '../helpers'
import type { ToolFormatter } from '@components'

export const todoWriteFormatter: ToolFormatter = {
  canonical: 'todo_write',
  aliases: ['todo'],
  kind: ToolKind.Think,
  format({ args, state }) {
    const todos = pickList(args, 'todos') ?? []
    const count = todos.length
    const counts: Record<string, number> = {}
    for (const entry of todos) {
      if (entry && typeof entry === 'object') {
        const status = (entry as Record<string, unknown>).status
        if (typeof status === 'string') {
          counts[status] = (counts[status] ?? 0) + 1
        }
      }
    }
    const bits = Object.entries(counts).map(([k, v]) => `${k}:${v}`)

    return {
      label: 'Todo write',
      arg: `${count} ${count === 1 ? 'item' : 'items'}`,
      detail: bits.length > 0 ? bits.join(' ') : undefined,
      state,
      kind: this.kind
    }
  }
}
