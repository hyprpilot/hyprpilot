import { faGlobe } from '@fortawesome/free-solid-svg-icons'

import { pickArgs, textBlocks } from '../shared'
import { PermissionUi, PillKind, ToolType } from '@constants/ui'
import type { Formatter } from '@interfaces/ui'
import { resolveShikiLang } from '@lib/markdown/mime'

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

/**
 * Sniff the MIME from a fetched body — first 1KB scan. Cheap
 * detection covering the common cases the captain hits (JSON,
 * Markdown, HTML); falls through to plaintext when nothing matches.
 *
 * Daemon-side fetch tools don't always thread the response
 * `Content-Type` onto the wire; this is the UI-side last resort
 * before the body lands as an unstyled `<pre>`.
 */
function sniffMime(body: string): string | undefined {
  const trimmed = body.trimStart()

  if (!trimmed) {
    return undefined
  }
  const head = trimmed.slice(0, 1024)

  if (head.startsWith('{') || head.startsWith('[')) {
    return 'application/json'
  }

  if (head.startsWith('<!DOCTYPE') || head.startsWith('<html') || head.startsWith('<HTML')) {
    return 'text/html'
  }

  if (head.startsWith('<?xml') || head.startsWith('<rss')) {
    return 'application/xml'
  }

  if (head.startsWith('# ') || head.startsWith('## ') || head.includes('\n# ') || head.includes('\n## ')) {
    return 'text/markdown'
  }

  return undefined
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

    // Render the response body via Shiki when we can sniff a
    // language. JSON / Markdown / HTML / XML all benefit from
    // syntax highlighting; plaintext (or unknown shapes) keep the
    // legacy `output` plain-pre rendering.
    const body = textBlocks(ctx.raw.content)
    let description: string | undefined
    let output: string | undefined

    if (body) {
      const mime = sniffMime(body)
      const lang = resolveShikiLang(mime, undefined)

      if (lang) {
        description = '```' + lang + '\n' + body + '\n```'
      } else {
        output = body
      }
    }

    return {
      id: ctx.raw.id,
      type: ToolType.WebFetch,
      name: ctx.name,
      state: ctx.state,
      icon: faGlobe,
      pill: PillKind.Default,
      permissionUi: PermissionUi.Row,
      title,
      ...(description ? { description } : {}),
      ...(output ? { output } : {}),
      ...(ctx.raw.rawInput ? { rawInput: ctx.raw.rawInput } : {})
    }
  }
}

export default webFetchFallback
