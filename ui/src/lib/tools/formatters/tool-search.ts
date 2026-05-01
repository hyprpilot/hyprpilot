import { ToolKind } from '@components'

import { pickNumber, pickString } from '../helpers'
import type { ToolFormatter } from '@components'

/**
 * Claude Code's `ToolSearch` — fetches tool schemas by keyword
 * (`query`) so the agent can discover deferred MCP / claude-code
 * built-ins lazily. Wire shape: `{ query: "<keywords>", max_results?:
 * number }`. Without a dedicated formatter the chip falls into
 * `fallbackFormatter` and renders the raw JSON — same noise as
 * the `Skill` case before we wired its formatter.
 *
 * `arg` carries the search query (the user-meaningful field);
 * `detail` surfaces `max_results` only when it's a non-default
 * value so a stock `5` doesn't crowd the chip.
 */
export const toolSearchFormatter: ToolFormatter = {
  canonical: 'tool_search',
  aliases: ['toolsearch'],
  label: 'Tool search',
  kind: ToolKind.Search,
  format({ args, state }) {
    const query = pickString(args, 'query')
    const max = pickNumber(args, 'maxresults', 'max_results')
    const detail = max !== undefined && max > 0 && max !== 5 ? `max ${max}` : undefined

    return {
      label: 'Tool search',
      arg: query,
      detail,
      state,
      kind: this.kind
    }
  }
}
