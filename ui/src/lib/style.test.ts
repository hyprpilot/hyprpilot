import { describe, expect, it } from 'vitest'

import { cn } from '@lib'

describe('cn', () => {
  it('merges plain class strings', () => {
    expect(cn('a', 'b')).toBe('a b')
  })

  it('dedupes conflicting tailwind utilities via tailwind-merge', () => {
    expect(cn('p-2', 'p-4')).toBe('p-4')
  })

  it('drops falsy values from clsx input', () => {
    expect(cn('a', false, null, undefined, 'b')).toBe('a b')
  })
})
