/**
 * Wire tool name → canonical key for the formatter map. Delegates to
 * `change-case::snakeCase` so claude-agent-acp's PascalCase wire
 * names (`MultiEdit`, `WebFetch`) collapse to the snake_case form
 * the formatter map keys on (`multi_edit`, `web_fetch`).
 *
 * MCP names (`mcp__server__leaf`) bypass `snakeCase` — its split
 * normalisation collapses `__` to `_`, which would clobber the
 * double-underscore prefix the dispatch uses to detect MCP tools.
 */

import { snakeCase } from 'change-case'

export function canonicalise(name: string | undefined): string {
  if (!name) {
    return ''
  }
  const trimmed = name.trim()

  if (trimmed.toLowerCase().startsWith('mcp__')) {
    return trimmed.toLowerCase()
  }

  return snakeCase(trimmed)
}
