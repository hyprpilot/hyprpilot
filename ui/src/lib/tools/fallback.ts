/**
 * Top-level fallback — runs when no `formatters[name]` matches and
 * the wire name doesn't carry the `mcp__` prefix. Renders with the
 * raw wire name as title (no synthetic 'other' label, no canonical
 * trickery). For codex-acp tool calls the wire title is already a
 * computed prose string ("Read foo.ts", "Search query in path") so
 * passing it through as-is reads correctly without per-adapter
 * branching.
 *
 * `description` extracts to `view.description` (markdown) so it
 * doesn't also surface as a field row; remaining args project onto
 * `fields`. Output dedupes against the description (some adapters
 * emit the description as both `rawInput.description` and a Text
 * content block).
 */

import { faPlug } from '@fortawesome/free-solid-svg-icons'

import { argsToFields, pickArgs, textBlocks } from './shared'
import { PermissionUi, PillKind, ToolType } from '@constants/ui'
import type { Formatter } from '@interfaces/ui'

const fallback: Formatter = {
  type: ToolType.Other,
  format(ctx) {
    const { description } = pickArgs(ctx.args, { description: 'string' })
    const fields = argsToFields(ctx.args, ['description'])
    const blockText = textBlocks(ctx.raw.content).trim()
    const output = blockText && blockText !== description?.trim() ? blockText : undefined
    const title = ctx.name.trim() || 'tool'

    return {
      id: ctx.raw.id,
      type: ToolType.Other,
      name: ctx.name,
      state: ctx.state,
      icon: faPlug,
      pill: PillKind.Default,
      permissionUi: PermissionUi.Row,
      title,
      ...(description ? { description } : {}),
      ...(fields ? { fields } : {}),
      ...(output ? { output } : {}),
      ...(ctx.raw.rawInput ? { rawInput: ctx.raw.rawInput } : {})
    }
  }
}

export default fallback
