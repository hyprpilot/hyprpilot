import { describe, expect, it } from 'vitest'

import { useMultiSelect } from './use-multi-select'

describe('useMultiSelect', () => {
  it('starts empty without seed', () => {
    const m = useMultiSelect()
    expect(m.ticked.value.size).toBe(0)
  })

  it('seeds the initial set', () => {
    const m = useMultiSelect(['a', 'b'])
    expect(m.isTicked('a')).toBe(true)
    expect(m.isTicked('b')).toBe(true)
    expect(m.isTicked('c')).toBe(false)
  })

  it('toggles ids in/out and replaces the Set instance for reactivity', () => {
    const m = useMultiSelect()
    const before = m.ticked.value
    m.toggle('a')
    expect(m.isTicked('a')).toBe(true)
    expect(m.ticked.value).not.toBe(before)
    m.toggle('a')
    expect(m.isTicked('a')).toBe(false)
  })

  it('reset() empties the set', () => {
    const m = useMultiSelect(['a', 'b'])
    m.reset()
    expect(m.ticked.value.size).toBe(0)
  })
})
