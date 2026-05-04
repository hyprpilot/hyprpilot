import { faPen } from '@fortawesome/free-solid-svg-icons'

import { diffBlocks, inferMimeFromPath, pickArgs, shortPath, textBlocks } from '../shared'
import { PermissionUi, PillKind, ToolType } from '@constants/ui'
import type { Formatter } from '@interfaces/ui'
import { cheapDiffMarkdown, richDiffMarkdown } from '@lib/diff'

const editFallback: Formatter = {
  type: ToolType.Edit,
  format(ctx) {
    const { filepath, path, replaceall } = pickArgs(ctx.args, {
      filepath: 'string',
      path: 'string',
      replaceall: 'boolean'
    })
    const p = filepath ?? path
    const trimmed = shortPath(p)
    const replaceAll = replaceall === true

    let title: string

    if (trimmed) {
      title = replaceAll ? `edit · ${trimmed} (replace all)` : `edit · ${trimmed}`
    } else {
      title = 'edit'
    }

    // ACP `ToolCallContent::Diff` carries `(path, oldText, newText)` —
    // pull every diff block off the wire and render via Shiki. The
    // permission flow uses the rich path (per-language syntax + diff
    // gutter via `transformerNotationDiff`); the post-completion pill
    // uses the cheap unified-diff path (lighter render on the high-
    // volume review surface).
    const diffs = diffBlocks(ctx.raw.content)
    let description: string | undefined

    if (diffs.length > 0) {
      const isPending = ctx.state === 'awaiting'
      const parts = diffs.map((d) => {
        const oldText = d.oldText ?? ''
        const mime = inferMimeFromPath(d.path)

        if (isPending) {
          return richDiffMarkdown(d.path, mime, oldText, d.newText).source
        }

        return cheapDiffMarkdown(d.path, oldText, d.newText)
      })

      description = parts.join('\n\n')
    }

    // `output` carries any text blocks (stdout / log lines from the
    // edit tool); diff blocks already routed into `description`.
    const output = textBlocks(ctx.raw.content)

    return {
      id: ctx.raw.id,
      type: ToolType.Edit,
      name: ctx.name,
      state: ctx.state,
      icon: faPen,
      pill: PillKind.Default,
      // ACP `kind: edit` → modal. Captain reviews diff in a focused
      // dialog; the inline row would clip the body. Routed by
      // formatter (not centrally) so future per-vendor overrides keep
      // local control of the surface.
      permissionUi: PermissionUi.Modal,
      title,
      ...(description ? { description } : {}),
      ...(output ? { output } : {}),
      ...(ctx.raw.rawInput ? { rawInput: ctx.raw.rawInput } : {})
    }
  }
}

export default editFallback
