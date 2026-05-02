import { describe, expect, it } from 'vitest'

import { parseTokens } from './parse-tokens'

describe('parseTokens', () => {
  it('parses a single token with scheme + value', () => {
    const out = parseTokens('see #{skills://git-commit} for context')

    expect(out).toHaveLength(1)
    expect(out[0]?.scheme).toBe('skills')
    expect(out[0]?.value).toBe('git-commit')
    expect(out[0]?.start).toBe(4)
    expect(out[0]?.end).toBe(26)
  })

  it('parses multiple tokens of mixed schemes', () => {
    const out = parseTokens('#{skills://a} mid #{prompt://b} end')

    expect(out.map((t) => `${t.scheme}://${t.value}`)).toEqual(['skills://a', 'prompt://b'])
  })

  it('rejects malformed schemes / values (uppercase, dots, spaces in scheme)', () => {
    expect(parseTokens('#{Skills://x}')).toEqual([])
    expect(parseTokens('#{has space://x}')).toEqual([])
  })

  it('returns empty for plain text', () => {
    expect(parseTokens('hello world')).toEqual([])
  })

  it('handles back-to-back tokens', () => {
    const out = parseTokens('#{skills://one}#{prompt://two}')

    expect(out.map((t) => t.scheme)).toEqual(['skills', 'prompt'])
  })
})
