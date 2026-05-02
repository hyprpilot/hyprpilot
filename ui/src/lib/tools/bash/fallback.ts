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
 * `description` always projects onto `view.description` (rendered as
 * markdown by every consumer); `command` projects onto `fields` as a
 * code-formatted row. If the wire's content blocks duplicate the
 * description text (claude-agent-acp emits the description as both
 * `rawInput.description` and a Text content block), the duplicate
 * `output` is suppressed.
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
    // the title would drown the chip header in code. Full command
    // is in the `command` field below; description (when set) lands
    // on the markdown body.
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

    if (command) {
      fields.push({ label: 'command', value: command })
    }

    if (id) {
      fields.push({ label: 'shell', value: id })
    }

    if (filter) {
      fields.push({ label: 'filter', value: filter })
    }

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
      ...(description ? { description } : {}),
      ...(fields.length > 0 ? { fields } : {}),
      ...(output ? { output } : {}),
      ...(ctx.raw.rawInput ? { rawInput: ctx.raw.rawInput } : {})
    }
  }
}

export default bashFallback
