import { faBookOpen } from '@fortawesome/free-solid-svg-icons'

import { diffBlocks, inferMimeFromPath, pickArgs, shortPath, textBlocks } from '../shared'
import { PermissionUi, PillKind, ToolType } from '@constants/ui'
import type { Formatter } from '@interfaces/ui'
import { cheapDiffMarkdown, richDiffMarkdown } from '@lib/diff'

const notebookEditFallback: Formatter = {
  type: ToolType.NotebookEdit,
  format(ctx) {
    const { notebookpath, filepath, cellid, editmode } = pickArgs(ctx.args, {
      notebookpath: 'string',
      filepath: 'string',
      cellid: 'string',
      editmode: 'string'
    })
    const p = notebookpath ?? filepath
    const bits: string[] = []

    if (cellid) {
      bits.push(`cell ${cellid}`)
    }

    if (editmode) {
      bits.push(editmode)
    }
    const suffix = bits.length > 0 ? ` (${bits.join(' · ')})` : ''
    const title = p ? `notebook · ${shortPath(p)}${suffix}` : `notebook${suffix}`

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
      type: ToolType.NotebookEdit,
      name: ctx.name,
      state: ctx.state,
      icon: faBookOpen,
      pill: PillKind.Default,
      permissionUi: PermissionUi.Modal,
      title,
      ...(description ? { description } : {}),
      ...(output ? { output } : {}),
      ...(ctx.raw.rawInput ? { rawInput: ctx.raw.rawInput } : {})
    }
  }
}

export default notebookEditFallback
