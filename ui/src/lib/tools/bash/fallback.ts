import { faTerminal } from '@fortawesome/free-solid-svg-icons'

import { pickArgs, textBlocks } from '../shared'
import { PermissionUi, PillKind, ToolType } from '@constants/ui'
import type { Formatter, ToolField } from '@interfaces/ui'

/**
 * Bash + bash_output — paired in `formatters` (both wire names route
 * here). `bash` runs a command; `bash_output` reads the tail of a
 * specific previously-spawned shell. Same icon, same tone, same
 * formatter — title disambiguates.
 *
 * The command body lands inside `description` as a fenced
 * ```` ```bash ```` block so Shiki highlights it under the spec
 * sheet's markdown render path; the natural-language `description`
 * (when present) precedes the fence so the rendered body reads as
 * `<prose>\n\n```bash\n<cmd>\n``` `. `output` (`textBlocks(content)`)
 * is suppressed when it duplicates the description prose.
 */
const bashFallback: Formatter = {
  type: ToolType.Bash,
  format(ctx) {
    const { command, description, isbackground, bashid, shellid, filter } = pickArgs(ctx.args, {
      command: 'string',
      description: 'string',
      isbackground: 'boolean',
      bashid: 'string',
      shellid: 'string',
      filter: 'string'
    })
    const id = bashid ?? shellid

    // Title surfaces just the leading command token (`python3` /
    // `ls` / `pnpm`) — pasting a multi-line python-script body into
    // the title would drown the chip header in code. The full
    // command rides into `description` below as a fenced bash block.
    let title: string

    if (command) {
      const head = command.trim().split(/\s+/, 1)[0] ?? 'bash'

      title = isbackground ? `bash · ${head} (background)` : `bash · ${head}`
    } else if (id) {
      title = filter ? `bash · tail #${id} — filter ${filter}` : `bash · tail #${id}`
    } else {
      title = 'bash'
    }

    const fields: ToolField[] = []

    if (id) {
      fields.push({ label: 'shell', value: id })
    }

    if (filter) {
      fields.push({ label: 'filter', value: filter })
    }

    // Prose first, fenced bash second. Empty parts drop out so a
    // command-only call renders as just the highlighted block, and a
    // description-only call (rare) keeps its prose.
    const parts: string[] = []

    if (description) {
      parts.push(description)
    }

    if (command) {
      parts.push('```bash\n' + command + '\n```')
    }
    const body = parts.length > 0 ? parts.join('\n\n') : undefined

    const blockText = textBlocks(ctx.raw.content).trim()
    const output = blockText && blockText !== description?.trim() ? blockText : undefined

    return {
      id: ctx.raw.id,
      type: ToolType.Bash,
      name: ctx.name,
      state: ctx.state,
      icon: faTerminal,
      pill: PillKind.Default,
      permissionUi: PermissionUi.Row,
      title,
      ...(body ? { description: body } : {}),
      ...(fields.length > 0 ? { fields } : {}),
      ...(output ? { output } : {}),
      ...(ctx.raw.rawInput ? { rawInput: ctx.raw.rawInput } : {})
    }
  }
}

export default bashFallback
