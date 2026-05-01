import { describe, expect, it } from 'vitest'

import { stripAnsi } from './ansi'

describe('stripAnsi', () => {
  it('passes plain text through', () => {
    expect(stripAnsi('hello\nworld')).toBe('hello\nworld')
  })

  it('strips colour escapes', () => {
    expect(stripAnsi('\x1b[31merror\x1b[0m: boom')).toBe('error: boom')
  })

  it('drops a line preceded by \\x1b[2K (clear-line)', () => {
    // Progress-bar style: start a line, then clear it before the next print.
    expect(stripAnsi('progress: 50%\x1b[2K\nfinal: done\n')).toBe('\nfinal: done\n')
  })

  it('leaves unrelated ANSI escapes alone', () => {
    // We deliberately do not handle every escape; non-color, non-clear-line
    // sequences pass through so the user can spot them and we don't lose data.
    const cursor = '\x1b[3;10H'
    expect(stripAnsi(`hi${cursor}there`)).toBe(`hi${cursor}there`)
  })

  it('returns empty string for empty input', () => {
    expect(stripAnsi('')).toBe('')
  })
})
