import { describe, expect, it } from 'vitest'

import { normalizeToolKind } from './index'
import { ToolKind } from '@constants/ui'

describe('normalizeToolKind', () => {
  it('maps structured MCP tool kinds to the generic UI kind', () => {
    expect(normalizeToolKind({
      type: 'mcp',
      server: 'memory',
      tool: 'read_graph'
    })).toBe(ToolKind.Other)
  })
})
