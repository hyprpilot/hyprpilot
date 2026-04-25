import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { __resetPaletteStackForTests, usePalette } from '@composables/palette'
import { useActiveInstance } from '@composables/use-active-instance'
import { __resetComposerForTests, useComposer } from '@composables/use-composer'
import { TauriCommand } from '@ipc'

const invokeMock = vi.fn()

vi.mock('@ipc', async () => {
  const actual = await vi.importActual<object>('@ipc')

  return {
    ...actual,
    invoke: (...args: unknown[]) => invokeMock(...args)
  }
})

import { openCommandsLeaf } from './palette-commands'

describe('openCommandsLeaf', () => {
  beforeEach(() => {
    __resetPaletteStackForTests()
    __resetComposerForTests()
    useActiveInstance().set('inst-1')
    invokeMock.mockReset()
  })

  afterEach(() => {
    __resetPaletteStackForTests()
    __resetComposerForTests()
  })

  it('renders no-instance placeholder when there is no active instance', async () => {
    // `useActiveInstance` only exposes a setter; '' is the falsy
    // sentinel the palette branch treats as "no live instance".
    useActiveInstance().set('')

    await openCommandsLeaf()

    const { stack } = usePalette()
    expect(stack.value).toHaveLength(1)
    expect(stack.value[0]?.entries[0]?.id).toBe('__no-instance__')
    expect(invokeMock).not.toHaveBeenCalled()
  })

  it('lists commands and inserts /<name> at the composer caret on commit', async () => {
    invokeMock.mockResolvedValue({
      commands: [
        { name: 'commit', description: 'create a git commit' },
        { name: 'review' }
      ]
    })

    await openCommandsLeaf()

    expect(invokeMock).toHaveBeenCalledWith(TauriCommand.CommandsList, { instanceId: 'inst-1' })

    const { stack } = usePalette()
    expect(stack.value).toHaveLength(1)
    const spec = stack.value[0]
    expect(spec?.entries.map((e) => e.name)).toEqual(['/commit', '/review'])

    const composer = useComposer()
    composer.text.value = 'foo bar'
    spec?.onCommit([spec.entries[0]!])

    expect(composer.text.value).toBe('foo bar/commit ')
  })

  it('does not insert anything when committing on a placeholder row', async () => {
    invokeMock.mockResolvedValue({ commands: [] })

    await openCommandsLeaf()

    const { stack } = usePalette()
    const spec = stack.value[0]
    expect(spec?.entries[0]?.id).toBe('__empty__')

    const composer = useComposer()
    composer.text.value = 'foo'
    spec?.onCommit([spec.entries[0]!])

    expect(composer.text.value).toBe('foo')
  })

  it('falls through to the error-state palette when invoke rejects', async () => {
    invokeMock.mockRejectedValue(new Error('commands/list not implemented — ref K-251'))

    await openCommandsLeaf()

    const { stack } = usePalette()
    expect(stack.value).toHaveLength(1)
    expect(stack.value[0]?.entries[0]?.id).toBe('__error__')
  })
})
