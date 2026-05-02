/**
 * MCP wire-name parsing. Sole site of `mcp__server__leaf` parsing in
 * the codebase — every consumer (the MCP formatter, future per-server
 * dispatch) reaches for `parseMcp` instead of duplicating split logic.
 */

export interface McpName {
  server: string
  leaf: string
}

export function isMcpName(canonical: string): boolean {
  return canonical.startsWith('mcp__')
}

export function parseMcp(canonical: string): McpName | undefined {
  if (!isMcpName(canonical)) {
    return undefined
  }
  const parts = canonical.split('__')

  if (parts.length < 3) {
    return undefined
  }
  const server = parts[1] ?? ''
  const leaf = parts.slice(2).join('__')

  if (!server || !leaf) {
    return undefined
  }

  return { server, leaf }
}

/** Render a snake_case-or-similar MCP leaf as a humanised phrase. */
export function humaniseLeaf(leaf: string): string {
  return leaf.replace(/_/g, ' ').trim()
}
