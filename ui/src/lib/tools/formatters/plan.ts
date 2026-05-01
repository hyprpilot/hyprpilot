import { ToolKind } from '@components'

import { pickString } from '../helpers'
import type { ToolFormatter } from '@components'

function truncatePlan(raw: string): string | undefined {
  if (!raw) {
    return undefined
  }
  if (raw.length <= 160) {
    return raw
  }

  return `${raw.slice(0, 159)}…`
}

export const planEnterFormatter: ToolFormatter = {
  canonical: 'plan_enter',
  aliases: ['enterplanmode'],
  kind: ToolKind.Think,
  format({ args, state }) {
    return {
      label: 'Plan enter',
      arg: truncatePlan(pickString(args, 'plan')),
      state,
      kind: this.kind
    }
  }
}

export const planExitFormatter: ToolFormatter = {
  canonical: 'plan_exit',
  aliases: ['exitplanmode'],
  kind: ToolKind.Think,
  format({ args, state }) {
    return {
      label: 'Plan exit',
      arg: truncatePlan(pickString(args, 'plan')),
      state,
      kind: this.kind
    }
  }
}
