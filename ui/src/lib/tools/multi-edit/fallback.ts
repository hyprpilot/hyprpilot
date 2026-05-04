import { faPen } from '@fortawesome/free-solid-svg-icons'

import { diffBlocks, inferMimeFromPath, pickArgs, shortPath, textBlocks } from '../shared'
import { PermissionUi, PillKind, ToolType } from '@constants/ui'
import type { Formatter } from '@interfaces/ui'
import { cheapDiffMarkdown, richDiffMarkdown } from '@lib/diff'

const multiEditFallback: Formatter = {
  type: ToolType.MultiEdit,
  format(ctx) {
    const { filepath, path, edits } = pickArgs(ctx.args, {
      filepath: 'string',
      path: 'string',
      edits: 'list'
    })
    const p = filepath ?? path
    const count = edits?.length ?? 0
    const title = p ? `edit · ${shortPath(p)}` : 'edit'
    const stat = count > 0 ? `${count} ${count === 1 ? 'edit' : 'edits'}` : undefined

    // ACP `Diff` content blocks → fenced markdown body. Pre-decision
    // (pending) gets the rich per-language render; post-completion
    // gets the cheap unified-diff path.
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
      type: ToolType.MultiEdit,
      name: ctx.name,
      state: ctx.state,
      icon: faPen,
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

export default multiEditFallback
