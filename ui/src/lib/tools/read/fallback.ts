import { faFileLines } from '@fortawesome/free-solid-svg-icons'

import { inferMimeFromPath, pickArgs, shortPath, textBlocks } from '../shared'
import { PermissionUi, PillKind, ToolType } from '@constants/ui'
import type { Formatter } from '@interfaces/ui'
import { resolveShikiLang } from '@lib/markdown/mime'

const readFallback: Formatter = {
  type: ToolType.Read,
  format(ctx) {
    const { filepath, path, offset, limit } = pickArgs(ctx.args, {
      filepath: 'string',
      path: 'string',
      offset: 'number',
      limit: 'number'
    })
    const p = filepath ?? path
    const trimmed = shortPath(p)

    let title: string

    if (trimmed) {
      if (offset !== undefined && limit !== undefined) {
        title = `read · ${trimmed} (lines ${offset}..${offset + limit})`
      } else if (offset !== undefined) {
        title = `read · ${trimmed} (from ${offset})`
      } else {
        title = `read · ${trimmed}`
      }
    } else {
      title = 'read'
    }

    // Per ACP, Read tool calls return the file body as a `text`
    // content block. Render it as a fenced code block in
    // `description` so the MarkdownBody chrome (collapse, copy,
    // Shiki) takes over — captain reads file content with the
    // file's syntax highlighting instead of an unstyled `<pre>`.
    // Language resolves from the path extension; the daemon
    // doesn't currently emit MIME on tool-call content, but
    // `inferMimeFromPath` gives a useful hint.
    const body = textBlocks(ctx.raw.content)
    let description: string | undefined

    if (body) {
      const mime = inferMimeFromPath(p ?? '')
      const lang = resolveShikiLang(mime, p) ?? 'plaintext'

      description = '```' + lang + '\n' + body + '\n```'
    }

    return {
      id: ctx.raw.id,
      type: ToolType.Read,
      name: ctx.name,
      state: ctx.state,
      icon: faFileLines,
      pill: PillKind.Default,
      permissionUi: PermissionUi.Row,
      title,
      ...(description ? { description } : {}),
      ...(ctx.raw.rawInput ? { rawInput: ctx.raw.rawInput } : {})
    }
  }
}

export default readFallback
