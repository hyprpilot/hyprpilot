import { ToolKind } from '@components'

import { parseMcpName, summariseArgs } from '../helpers'
import type { ToolFormatter } from '@components'

/**
 * Title-cased label per ACP wire kind. Used by the fallback
 * formatter so unknown tools (anything not in the registered
 * formatter list) get a human-readable text label instead of the
 * agent's prose title. The visual glyph is the FontAwesome icon
 * resolved from `kind` via `iconForToolKind`; this map feeds
 * `aria-label` / text-only surfaces. Mirrors the convention the
 * registered formatters follow (`Bash`, `Read`, `Edit`, …) — names
 * read like the agent's actual tool, not a verb the UI invents.
 */
const SHORT_BY_WIRE_KIND: Record<string, string> = {
  read: 'Read',
  edit: 'Edit',
  delete: 'Delete',
  move: 'Move',
  search: 'Search',
  execute: 'Bash',
  think: 'Think',
  fetch: 'Fetch',
  switch_mode: 'Switch mode'
}

/**
 * ACP wire kind → our `ToolKind` enum (drives chip tone + the body's
 * KIND row). `other` / unknown kinds collapse to `Acp` which the chip
 * renderer skips so the body doesn't show a meaningless "acp" line.
 */
const TOOL_KIND_BY_WIRE_KIND: Record<string, ToolKind> = {
  read: ToolKind.Read,
  edit: ToolKind.Write,
  delete: ToolKind.Write,
  move: ToolKind.Write,
  search: ToolKind.Search,
  execute: ToolKind.Bash,
  think: ToolKind.Think,
  fetch: ToolKind.Read,
  switch_mode: ToolKind.Agent
}

/**
 * Last-resort formatter used when the registry has no specific
 * formatter for `name`. Handles MCP-style names (`mcp__server__tool`)
 * by extracting the server as label and the tool leaf as arg.
 *
 * For everything else: route the label through the wire `kind` so
 * the chip's text identifier is a kind word (`Execute` / `Read` /
 * `Edit` / …) — never the agent's full prose title. The chip's
 * visual leading element is the FontAwesome icon resolved by
 * `iconForToolKind(chip.kind)`; this label feeds aria + text-only
 * surfaces. Title (when short) rides on `detail` for context; the
 * `arg` column carries the first string-valued input (command /
 * path / pattern / …).
 */
export const fallbackFormatter: ToolFormatter = {
  canonical: '__fallback__',
  kind: ToolKind.Acp,
  format({ name, rawName, args, state, kind }) {
    const mcp = parseMcpName(name)
    if (mcp) {
      const summary = summariseArgs(args)
      const toolLabel = mcp.tool.replace(/_/g, ' ')

      return {
        label: mcp.server,
        arg: summary ? `${toolLabel}(${summary})` : toolLabel,
        state,
        kind: ToolKind.Acp
      }
    }

    const wire = kind.toLowerCase()
    const label = SHORT_BY_WIRE_KIND[wire] ?? '·'
    const toolKind = TOOL_KIND_BY_WIRE_KIND[wire] ?? ToolKind.Acp
    const summary = summariseArgs(args)

    return {
      label,
      arg: summary || undefined,
      detail: rawName.length > 0 && rawName.length <= 60 ? rawName : undefined,
      state,
      kind: toolKind
    }
  }
}
