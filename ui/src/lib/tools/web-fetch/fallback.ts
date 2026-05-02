import { faGlobe } from '@fortawesome/free-solid-svg-icons'

import { pickArgs, textBlocks } from '../shared'
import { PermissionUi, PillKind, ToolType } from '@constants/ui'
import type { Formatter } from '@interfaces/ui'

function hostOf(url: string | undefined): string | undefined {
  if (!url) {
    return undefined
  }

  try {
    return new URL(url).host
  } catch {
    return url
  }
}

const webFetchFallback: Formatter = {
  type: ToolType.WebFetch,
  format(ctx) {
    const { url, uri, prompt } = pickArgs(ctx.args, {
      url: 'string',
      uri: 'string',
      prompt: 'string'
    })
    const target = url ?? uri
    const host = hostOf(target)
    let title: string

    if (host && prompt) {
      title = `fetch · ${host} — ${prompt}`
    } else if (host) {
      title = `fetch · ${host}`
    } else {
      title = 'fetch'
    }
    const output = textBlocks(ctx.raw.content)

    return {
      id: ctx.raw.id,
      type: ToolType.WebFetch,
      name: ctx.name,
      state: ctx.state,
      icon: faGlobe,
      pill: PillKind.Default,
      permissionUi: PermissionUi.Row,
      title,
      ...(output ? { output } : {}),
      ...(ctx.raw.rawInput ? { rawInput: ctx.raw.rawInput } : {})
    }
  }
}

export default webFetchFallback
