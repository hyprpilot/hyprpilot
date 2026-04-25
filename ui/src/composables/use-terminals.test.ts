import { beforeEach, describe, expect, it } from 'vitest'

import { pushTerminalChunk, pushTerminalExit, resetTerminals, useTerminals } from '@composables/use-terminals'

beforeEach(() => {
  resetTerminals('A')
  resetTerminals('B')
})

describe('useTerminals', () => {
  it('isolates terminal entries between instances keyed by terminalId', () => {
    pushTerminalChunk('A', { terminalId: 't-1', data: 'file1\n', command: 'ls' })
    pushTerminalChunk('B', { terminalId: 't-2', data: '/root\n', command: 'pwd' })

    const a = useTerminals('A').all.value
    const b = useTerminals('B').all.value

    expect(a.map((e) => e.id)).toEqual(['t-1'])
    expect(a[0]?.output).toBe('file1\n')
    expect(a[0]?.command).toBe('ls')
    expect(b.map((e) => e.id)).toEqual(['t-2'])
    expect(b[0]?.output).toBe('/root\n')
  })

  it('appends chunks and resolves exit flips running off', () => {
    pushTerminalChunk('A', { terminalId: 't-1', data: 'line 1\n', command: 'tail -f' })
    pushTerminalChunk('A', { terminalId: 't-1', data: 'line 2\n' })
    pushTerminalExit('A', { terminalId: 't-1', exitCode: 0 })

    const entry = useTerminals('A').byId('t-1').value
    expect(entry?.output).toBe('line 1\nline 2\n')
    expect(entry?.running).toBe(false)
    expect(entry?.exitCode).toBe(0)
    expect(entry?.truncated).toBe(false)
  })

  it('truncates past MAX_LINES (2000) and sets truncated=true', () => {
    const lines = Array.from({ length: 2500 }, (_, i) => `L${i}`).join('\n') + '\n'
    pushTerminalChunk('A', { terminalId: 't-1', data: lines })

    const entry = useTerminals('A').byId('t-1').value
    expect(entry?.truncated).toBe(true)
    // Output retains exactly MAX_LINES + trailing newline boundary.
    const retained = entry?.output.split('\n') ?? []
    expect(retained.length).toBeLessThanOrEqual(2001)
    // Oldest line dropped, newest survives.
    expect(entry?.output.includes('L0\n')).toBe(false)
    expect(entry?.output.includes('L2499\n')).toBe(true)
  })

  it('byId returns undefined for unknown terminalId', () => {
    pushTerminalChunk('A', { terminalId: 't-1', data: 'hi' })
    expect(useTerminals('A').byId('missing').value).toBeUndefined()
  })

  it('records signal alongside exit', () => {
    pushTerminalChunk('A', { terminalId: 't-1', data: '...' })
    pushTerminalExit('A', { terminalId: 't-1', signal: 'SIGTERM' })

    const entry = useTerminals('A').byId('t-1').value
    expect(entry?.running).toBe(false)
    expect(entry?.signal).toBe('SIGTERM')
    expect(entry?.exitCode).toBeUndefined()
  })
})
