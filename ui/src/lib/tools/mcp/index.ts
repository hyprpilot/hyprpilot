import fallback from './fallback'
import { AdapterId } from '@constants/ui'
import type { Formatter, Formatters } from '@interfaces/ui'

/**
 * Per-MCP-server formatter overrides. Keyed by server name (NOT
 * adapter — MCP servers are user-configured globally; the same
 * `playwright` server emits identical shapes regardless of which
 * agent called it). Empty today; future server-specific formatters
 * land as `mcp/servers/<server>.ts` files registered here.
 */
const perServer: Record<string, Formatter> = {}

const mcpEntry: Formatters = (_adapter?: AdapterId) => ({
  type: fallback.type,
  format(ctx) {
    const lower = ctx.name.toLowerCase()

    if (lower.startsWith('mcp__')) {
      const parts = lower.split('__')
      const server = parts[1]

      if (server && perServer[server]) {
        return perServer[server].format(ctx)
      }
    }

    return fallback.format(ctx)
  }
})

export default mcpEntry
