import { beforeEach, describe, expect, it } from 'vitest'

import { __resetPaletteStackForTests, PaletteMode, type PaletteSpec, usePalette } from './palette'

function makeSpec(name: string): PaletteSpec {
  return {
    mode: PaletteMode.Select,
    title: name,
    entries: [{ id: name, name }],
    onCommit: () => {}
  }
}

beforeEach(() => {
  __resetPaletteStackForTests()
})

describe('usePalette', () => {
  it('open() pushes onto the stack', () => {
    const { stack, open } = usePalette()
    open(makeSpec('root'))
    open(makeSpec('child'))

    expect(stack.value).toHaveLength(2)
    expect(stack.value[0]?.title).toBe('root')
    expect(stack.value[1]?.title).toBe('child')
  })

  it('close() pops one level', () => {
    const { stack, open, close } = usePalette()
    open(makeSpec('root'))
    open(makeSpec('child'))
    close()

    expect(stack.value).toHaveLength(1)
    expect(stack.value[0]?.title).toBe('root')
  })

  it('closeAll() empties the stack', () => {
    const { stack, open, closeAll } = usePalette()
    open(makeSpec('a'))
    open(makeSpec('b'))
    open(makeSpec('c'))
    closeAll()

    expect(stack.value).toHaveLength(0)
  })

  it('close() on an empty stack is a no-op', () => {
    const { stack, close } = usePalette()
    close()

    expect(stack.value).toHaveLength(0)
  })

  it('stack reactivity — mutations reflect across usePalette() callers', () => {
    const a = usePalette()
    const b = usePalette()
    a.open(makeSpec('x'))

    expect(b.stack.value).toHaveLength(1)
    expect(b.stack.value[0]?.title).toBe('x')

    b.close()
    expect(a.stack.value).toHaveLength(0)
  })
})
