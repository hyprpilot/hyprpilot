import { describe, expect, it } from 'vitest'

import { iconForToolKind } from './types'

describe('iconForToolKind', () => {
  it('maps known kind strings case-insensitively', () => {
    expect(iconForToolKind('bash')).toEqual(['fas', 'terminal'])
    expect(iconForToolKind('BASH')).toEqual(['fas', 'terminal'])
    expect(iconForToolKind('Read')).toEqual(['fas', 'file-lines'])
  })

  it('falls back to the generic cube glyph for undefined kinds', () => {
    expect(iconForToolKind(undefined)).toEqual(['fas', 'cube'])
    expect(iconForToolKind('')).toEqual(['fas', 'cube'])
    expect(iconForToolKind('nonsense')).toEqual(['fas', 'cube'])
  })
})
