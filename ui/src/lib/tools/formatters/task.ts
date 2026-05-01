import { ToolKind } from '@components'

import { pickString } from '../helpers'
import type { ToolFormatter } from '@components'

export const taskFormatter: ToolFormatter = {
  canonical: 'task',
  aliases: ['agent'],
  kind: ToolKind.Agent,
  format({ args, state }) {
    const subagent = pickString(args, 'subagenttype') || 'agent'
    const description = pickString(args, 'description')
    const prompt = pickString(args, 'prompt')
    const truncated = prompt.length > 200 ? `${prompt.slice(0, 199)}…` : prompt
    const bits = [description, truncated].filter((s) => s.length > 0)

    return {
      label: 'Task',
      arg: subagent,
      detail: bits.length > 0 ? bits.join(' — ') : undefined,
      state,
      kind: this.kind
    }
  }
}
