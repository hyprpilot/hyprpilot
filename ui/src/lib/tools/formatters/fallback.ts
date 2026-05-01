import { ToolKind } from '@components'

import { parseMcpName, summariseArgs } from '../helpers'
import type { Args, ToolFormatter } from '@components'
import type { ToolField } from '@interfaces/ui/chat'

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
 * Render every string-typed entry of an MCP tool's `args` map as a
 * structured `ToolField`. Long values stay verbatim; the spec sheet
 * row layout caps width via overflow-wrap. Non-string values
 * (numbers, bools, objects) get JSON-stringified inline so the
 * captain still sees what was passed without a separate "raw JSON"
 * disclosure. Empty maps return undefined so the caller skips the
 * `fields` slot entirely.
 */
function fieldsFromArgs(args: Args): ToolField[] | undefined {
  const out: ToolField[] = []
  for (const [k, v] of Object.entries(args)) {
    if (v === undefined || v === null) {
      continue
    }
    let value: string
    if (typeof v === 'string') {
      if (v.length === 0) {
        continue
      }
      value = v
    } else if (typeof v === 'number' || typeof v === 'boolean') {
      value = String(v)
    } else {
      try {
        value = JSON.stringify(v)
      } catch {
        continue
      }
    }
    out.push({ label: k, value })
  }
  return out.length > 0 ? out : undefined
}

/**
 * Last-resort formatter used when the registry has no specific
 * formatter for `name`. Handles MCP-style names (`mcp__server__tool`)
 * by extracting the server as label, the tool leaf as the title
 * suffix, and projecting the args map onto structured field rows
 * (`path = …`, `pattern = …`) — captains scanning the expanded pill
 * see each input on its own labelled row.
 *
 * NOTE — `arg` is intentionally NOT set for MCP tools. The `arg`
 * column is shown as a `command` row in the expanded spec sheet
 * (load-bearing for Bash, where `arg` IS the shell command); for an
 * MCP call, the "first string-valued input" is just an arbitrary key
 * (e.g. `applicationName = rubik-monitoring`) — labelling it
 * `command` is a lie. The structured `fields` rows already show
 * every input with its real key, so the `command` row would
 * duplicate one of them under a misleading label.
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
  kind: ToolKind.Unknown,
  format({ name, rawName, args, state, kind }) {
    const mcp = parseMcpName(name)
    if (mcp) {
      const toolLabel = mcp.tool.replace(/_/g, ' ')
      const fields = fieldsFromArgs(args)

      return {
        label: mcp.server,
        // Suffix the leaf tool name as the chip's `title` so the
        // header reads `[plug] filesystem · read file`. Args ride
        // on the structured `fields` rows in the expanded body —
        // skip the misleading `arg → command` projection (see the
        // doc comment on this function for why).
        title: toolLabel,
        ...(fields ? { fields } : {}),
        state,
        kind: ToolKind.Unknown
      }
    }

    const wire = kind.toLowerCase()
    const label = SHORT_BY_WIRE_KIND[wire] ?? '·'
    const toolKind = TOOL_KIND_BY_WIRE_KIND[wire] ?? ToolKind.Unknown
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
