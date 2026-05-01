import { ToolKind } from '@components'

import { pickString, pickStringList } from '../helpers'
import type { ToolFormatter } from '@components'

export const webFetchFormatter: ToolFormatter = {
  canonical: 'web_fetch',
  kind: ToolKind.Acp,
  format({ args, state }) {
    const url = pickString(args, 'url', 'uri')
    const prompt = pickString(args, 'prompt')

    return {
      label: 'Web fetch',
      arg: url,
      detail: prompt || undefined,
      state,
      kind: this.kind
    }
  }
}

export const webSearchFormatter: ToolFormatter = {
  canonical: 'web_search',
  kind: ToolKind.Search,
  format({ args, state }) {
    const query = pickString(args, 'query')
    const allowed = pickStringList(args, 'alloweddomains')
    const blocked = pickStringList(args, 'blockeddomains')
    const bits: string[] = []
    if (allowed.length > 0) {
      bits.push(`allowed: ${allowed.join(', ')}`)
    }
    if (blocked.length > 0) {
      bits.push(`blocked: ${blocked.join(', ')}`)
    }

    return {
      label: 'Web search',
      arg: query,
      detail: bits.length > 0 ? bits.join(' · ') : undefined,
      state,
      kind: this.kind
    }
  }
}
