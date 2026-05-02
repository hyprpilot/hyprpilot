import { faPlug } from '@fortawesome/free-solid-svg-icons'

import { argsToFields, humaniseLeaf, parseMcp, pickArgs, textBlocks } from '../shared'
import { PermissionUi, PillKind, ToolType } from '@constants/ui'
import type { Formatter } from '@interfaces/ui'

/**
 * Generic MCP fallback — covers every `mcp__server__leaf` tool. Args
 * project onto structured field rows; output text passes through to
 * `output` (NOT `description` — markdown safety per D7). Per-server
 * specialised formatters can drop into `mcp/servers/<server>.ts`
 * later; the dispatch in `mcp/index.ts` will pick them up.
 */
const mcpFallback: Formatter = {
  type: ToolType.Mcp,
  format(ctx) {
    const parsed = parseMcp(ctx.name.toLowerCase())
    const server = parsed?.server ?? 'mcp'
    const leaf = parsed ? humaniseLeaf(parsed.leaf) : ctx.name
    const title = `${server} · ${leaf}`
    const { description } = pickArgs(ctx.args, { description: 'string' })
    const fields = argsToFields(ctx.args, ['description'])
    const blockText = textBlocks(ctx.raw.content).trim()
    const output = blockText && blockText !== description?.trim() ? blockText : undefined

    return {
      id: ctx.raw.id,
      type: ToolType.Mcp,
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

export default mcpFallback
