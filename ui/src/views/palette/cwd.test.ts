import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { nextTick } from 'vue'

import { TauriCommand } from '@ipc'

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }))

vi.mock('@ipc/bridge', async () => ({
  ...(await vi.importActual<object>('@ipc/bridge')),
  invoke: (command: string, args?: Record<string, unknown>) => invoke(command, args),
  listen: vi.fn()
}))

import { useActiveInstance } from '@composables'
import { __resetCwdHistoryForTests, useCwdHistory } from '@composables'
import { __resetHomeDirForTests, useHomeDir } from '@composables'
import { __resetPaletteStackForTests, type PaletteEntry, usePalette } from '@composables'

import { openCwdLeaf } from './cwd'

beforeEach(() => {
  invoke.mockReset()
  __resetCwdHistoryForTests()
  __resetPaletteStackForTests()
  __resetHomeDirForTests()
  useActiveInstance().id.value = undefined
})

afterEach(() => {
  __resetPaletteStackForTests()
})

describe('openCwdLeaf', () => {
  it('opens a single palette layer titled "cwd" with one manual sentinel by default', () => {
    openCwdLeaf()
    const { stack } = usePalette()

    expect(stack.value).toHaveLength(1)
    expect(stack.value[0]?.title).toBe('cwd')
    expect(stack.value[0]?.entries).toHaveLength(1)
    expect(stack.value[0]?.entries[0]?.id).toBe('cwd-manual')
  })

  it('lists recent history entries followed by the manual sentinel', () => {
    const { push } = useCwdHistory()
    push('/tmp/a')
    push('/tmp/b')

    openCwdLeaf()
    const entries = usePalette().stack.value[0]?.entries ?? []
    const ids = entries.map((e: PaletteEntry) => e.id)

    expect(ids).toEqual(['cwd-recent:/tmp/b', 'cwd-recent:/tmp/a', 'cwd-manual'])
  })

  it('committing a recent row invokes instance_restart with that path', async () => {
    useActiveInstance().set('inst-1')
    const { push } = useCwdHistory()
    push('/home/cenk/dev')
    invoke.mockResolvedValue({ id: 'inst-1' })

    openCwdLeaf()
    const spec = usePalette().stack.value[0]
    const recent = spec?.entries.find((e: PaletteEntry) => e.id === 'cwd-recent:/home/cenk/dev')
    expect(recent).toBeTruthy()

    await spec?.onCommit([recent as PaletteEntry], '')
    await nextTick()

    expect(invoke).toHaveBeenCalledWith(TauriCommand.InstanceRestart, {
      instanceId: 'inst-1',
      cwd: '/home/cenk/dev'
    })
  })

  it('committing the manual row uses the live query as the path', async () => {
    useActiveInstance().set('inst-1')
    invoke.mockResolvedValue({ id: 'inst-1' })

    openCwdLeaf()
    const spec = usePalette().stack.value[0]
    const manual = spec?.entries.find((e: PaletteEntry) => e.id === 'cwd-manual')
    expect(manual).toBeTruthy()

    await spec?.onCommit([manual as PaletteEntry], '/srv/projects/x')
    await nextTick()

    expect(invoke).toHaveBeenCalledWith(TauriCommand.InstanceRestart, {
      instanceId: 'inst-1',
      cwd: '/srv/projects/x'
    })
  })

  it('expands `~/path` against the resolved home dir before submit', async () => {
    useActiveInstance().set('inst-1')
    useHomeDir().homeDir.value = '/home/cenk'
    invoke.mockResolvedValue({ id: 'inst-1' })

    openCwdLeaf()
    const spec = usePalette().stack.value[0]
    const manual = spec?.entries.find((e: PaletteEntry) => e.id === 'cwd-manual')

    await spec?.onCommit([manual as PaletteEntry], '~/dev/x')

    expect(invoke).toHaveBeenCalledWith(TauriCommand.InstanceRestart, {
      instanceId: 'inst-1',
      cwd: '/home/cenk/dev/x'
    })
  })

  it('rejects non-absolute paths client-side without invoking restart', async () => {
    useActiveInstance().set('inst-1')

    openCwdLeaf()
    const spec = usePalette().stack.value[0]
    const manual = spec?.entries.find((e: PaletteEntry) => e.id === 'cwd-manual')

    await spec?.onCommit([manual as PaletteEntry], 'relative/path')

    expect(invoke).not.toHaveBeenCalled()
  })

  it('rejects empty manual input without invoking restart', async () => {
    useActiveInstance().set('inst-1')

    openCwdLeaf()
    const spec = usePalette().stack.value[0]
    const manual = spec?.entries.find((e: PaletteEntry) => e.id === 'cwd-manual')

    await spec?.onCommit([manual as PaletteEntry], '   ')

    expect(invoke).not.toHaveBeenCalled()
  })

  it('refuses to commit when no active instance exists', async () => {
    openCwdLeaf()
    const spec = usePalette().stack.value[0]
    const manual = spec?.entries.find((e: PaletteEntry) => e.id === 'cwd-manual')

    await spec?.onCommit([manual as PaletteEntry], '/tmp/x')

    expect(invoke).not.toHaveBeenCalled()
  })

  it('successful commit pushes the (expanded) cwd onto history', async () => {
    useActiveInstance().set('inst-1')
    invoke.mockResolvedValue({ id: 'inst-1' })

    openCwdLeaf()
    const spec = usePalette().stack.value[0]
    const manual = spec?.entries.find((e: PaletteEntry) => e.id === 'cwd-manual')

    await spec?.onCommit([manual as PaletteEntry], '/var/log')
    await nextTick()

    expect(useCwdHistory().history.value[0]).toBe('/var/log')
  })

  it('failed restart does not push to history', async () => {
    useActiveInstance().set('inst-1')
    invoke.mockRejectedValue(new Error('cwd not a directory'))

    openCwdLeaf()
    const spec = usePalette().stack.value[0]
    const manual = spec?.entries.find((e: PaletteEntry) => e.id === 'cwd-manual')

    await spec?.onCommit([manual as PaletteEntry], '/nonexistent')
    await nextTick()

    expect(useCwdHistory().history.value).toEqual([])
  })
})
