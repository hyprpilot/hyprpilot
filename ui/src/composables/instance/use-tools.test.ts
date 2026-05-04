import { beforeEach, describe, expect, it } from 'vitest'

import { pushToolCall, resetTools, useTools } from '@composables'

const stub = {
  title: 'read',
  fields: []
}

beforeEach(() => {
  resetTools('A')
  resetTools('B')
})

describe('useTools', () => {
  it('isolates tool calls between instances', () => {
    pushToolCall('A', 'agent-A', 's-a', {
      sessionUpdate: 'tool_call',
      toolCallId: 'tc-1',
      title: 'read',
      status: 'completed',
      formatted: stub
    })
    pushToolCall('B', 'agent-B', 's-b', {
      sessionUpdate: 'tool_call',
      toolCallId: 'tc-2',
      title: 'write',
      status: 'pending',
      formatted: stub
    })

    const a = useTools('A').calls.value
    const b = useTools('B').calls.value

    expect(a).toHaveLength(1)
    expect(a[0]?.toolCallId).toBe('tc-1')
    expect(b).toHaveLength(1)
    expect(b[0]?.toolCallId).toBe('tc-2')
  })

  it('merges tool_call_update onto the existing entry by toolCallId', () => {
    pushToolCall('A', 'agent-A', 's-a', {
      sessionUpdate: 'tool_call',
      toolCallId: 'tc-1',
      title: 'read',
      status: 'pending',
      formatted: stub
    })
    pushToolCall('A', 'agent-A', 's-a', {
      sessionUpdate: 'tool_call_update',
      toolCallId: 'tc-1',
      status: 'completed',
      formatted: stub
    })

    const calls = useTools('A').calls.value

    expect(calls).toHaveLength(1)
    expect(calls[0]?.status).toBe('completed')
    expect(calls[0]?.title).toBe('read')
  })
})
