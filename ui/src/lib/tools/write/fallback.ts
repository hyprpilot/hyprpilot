import { faPenToSquare } from '@fortawesome/free-solid-svg-icons'

import { diffBlocks, inferMimeFromPath, pickArgs, shortPath, textBlocks } from '../shared'
import { PermissionUi, PillKind, ToolType } from '@constants/ui'
import type { Formatter } from '@interfaces/ui'
import { cheapDiffMarkdown, richDiffMarkdown } from '@lib/diff'

const writeFallback: Formatter = {
  type: ToolType.Write,
  format(ctx) {
    const { filepath, path, content, newstring } = pickArgs(ctx.args, {
      filepath: 'string',
      path: 'string',
      content: 'string',
      newstring: 'string'
    })
    const p = filepath ?? path
    const body = content ?? newstring
    const title = p ? `write · ${shortPath(p)}` : 'write'
    const stat = body && body.length > 0 ? `${body.length} chars` : undefined

    const diffs = diffBlocks(ctx.raw.content)
    let description: string | undefined

    if (diffs.length > 0) {
      const isPending = ctx.state === 'awaiting'
      const parts = diffs.map((d) => {
        const oldText = d.oldText ?? ''
        const mime = inferMimeFromPath(d.path)

        return isPending ? richDiffMarkdown(d.path, mime, oldText, d.newText).source : cheapDiffMarkdown(d.path, oldText, d.newText)
      })

      description = parts.join('\n\n')
    }
    const output = textBlocks(ctx.raw.content)

    return {
      id: ctx.raw.id,
      type: ToolType.Write,
      name: ctx.name,
      state: ctx.state,
      icon: faPenToSquare,
      pill: PillKind.Default,
      permissionUi: PermissionUi.Modal,
      title,
      ...(stat ? { stat } : {}),
      ...(description ? { description } : {}),
      ...(output ? { output } : {}),
      ...(ctx.raw.rawInput ? { rawInput: ctx.raw.rawInput } : {})
    }
  }
}

export default writeFallback
