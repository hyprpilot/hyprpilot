import { beforeEach, describe, expect, it } from 'vitest'

import { pushTerminalChunk, resetTerminals, useTerminals } from '@composables/use-terminals'

beforeEach(() => {
  resetTerminals('A')
  resetTerminals('B')
})

describe('useTerminals', () => {
  it('isolates terminal streams between instances keyed by toolCallId', () => {
    pushTerminalChunk('A', { toolCallId: 'tc-1', sessionId: 's-a', command: 'ls', stdout: 'file1\n' })
    pushTerminalChunk('B', { toolCallId: 'tc-2', sessionId: 's-b', command: 'pwd', stdout: '/root\n' })

    const a = useTerminals('A').streams.value
    const b = useTerminals('B').streams.value

    expect(Object.keys(a)).toEqual(['tc-1'])
    expect(a['tc-1']?.stdout).toBe('file1\n')
    expect(Object.keys(b)).toEqual(['tc-2'])
    expect(b['tc-2']?.stdout).toBe('/root\n')
  })

  it('appends stdout chunks to the existing stream', () => {
    pushTerminalChunk('A', { toolCallId: 'tc-1', sessionId: 's-a', command: 'tail -f', stdout: 'line 1\n' })
    pushTerminalChunk('A', { toolCallId: 'tc-1', sessionId: 's-a', stdout: 'line 2\n' })
    pushTerminalChunk('A', { toolCallId: 'tc-1', sessionId: 's-a', stdout: 'line 3\n', running: false, exitCode: 0 })

    const stream = useTerminals('A').streams.value['tc-1']
    expect(stream?.stdout).toBe('line 1\nline 2\nline 3\n')
    expect(stream?.running).toBe(false)
    expect(stream?.exitCode).toBe(0)
  })
})
