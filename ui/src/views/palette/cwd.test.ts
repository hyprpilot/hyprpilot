import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { nextTick } from 'vue'

import { openCwdLeaf } from './cwd'
import { useActiveInstance, __resetCwdHistoryForTests, useCwdHistory, __resetHomeDirForTests, useHomeDir } from '@composables'
import { __resetPaletteStackForTests, type PaletteEntry, usePalette, PaletteMode } from '@composables'
import { TauriCommand } from '@ipc'

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }))

vi.mock('@ipc/bridge', async() => ({
  ...(await vi.importActual<object>('@ipc/bridge')),
  invoke: (command: string, args?: Record<string, unknown>) => invoke(command, args),
  listen: vi.fn()
}))

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
  it('opens an Input-mode palette layer titled "cwd" with no rows by default', () => {
    openCwdLeaf()
    const { stack } = usePalette()

    expect(stack.value).toHaveLength(1)
    expect(stack.value[0]?.title).toBe('cwd')
    expect(stack.value[0]?.mode).toBe(PaletteMode.Input)
    // No history → no rows. Input mode hides the "no matches"
    // empty-state, captain just sees the bare input.
    expect(stack.value[0]?.entries).toHaveLength(0)
  })

  it('lists recent history entries when history is non-empty', () => {
    const { push } = useCwdHistory()

    push('/tmp/a')
    push('/tmp/b')

    openCwdLeaf()
    const entries = usePalette().stack.value[0]?.entries ?? []
    const ids = entries.map((e: PaletteEntry) => e.id)

    // MRU order — last-pushed first; no manual-sentinel row anymore.
    expect(ids).toEqual(['cwd-recent:/tmp/b', 'cwd-recent:/tmp/a'])
  })

  it('committing a recent row invokes instance_restart with that path', async() => {
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

  it('committing with no highlighted row uses the live query as the path', async() => {
    useActiveInstance().set('inst-1')
    invoke.mockResolvedValue({ id: 'inst-1' })

    openCwdLeaf()
    const spec = usePalette().stack.value[0]

    await spec?.onCommit([], '/srv/projects/x')
    await nextTick()

    expect(invoke).toHaveBeenCalledWith(TauriCommand.InstanceRestart, {
      instanceId: 'inst-1',
      cwd: '/srv/projects/x'
    })
  })

  it('expands `~/path` against the resolved home dir before submit', async() => {
    useActiveInstance().set('inst-1')
    useHomeDir().homeDir.value = '/home/cenk'
    invoke.mockResolvedValue({ id: 'inst-1' })

    openCwdLeaf()
    const spec = usePalette().stack.value[0]

    await spec?.onCommit([], '~/dev/x')

    expect(invoke).toHaveBeenCalledWith(TauriCommand.InstanceRestart, {
      instanceId: 'inst-1',
      cwd: '/home/cenk/dev/x'
    })
  })

  it('rejects relative paths when there is no active-instance cwd to resolve against', async() => {
    useActiveInstance().set('inst-1')

    openCwdLeaf()
    const spec = usePalette().stack.value[0]

    await spec?.onCommit([], 'relative/path')

    expect(invoke).not.toHaveBeenCalled()
  })

  it('rejects empty input without invoking restart', async() => {
    useActiveInstance().set('inst-1')

    openCwdLeaf()
    const spec = usePalette().stack.value[0]

    await spec?.onCommit([], '   ')

    expect(invoke).not.toHaveBeenCalled()
  })

  it('refuses to commit when no active instance exists', async() => {
    openCwdLeaf()
    const spec = usePalette().stack.value[0]

    await spec?.onCommit([], '/tmp/x')

    expect(invoke).not.toHaveBeenCalled()
  })

  it('resolves a relative path against the active instance cwd before invoking restart', async() => {
    const { setInstanceCwd } = await import('@composables')

    useActiveInstance().set('inst-1')
    setInstanceCwd('inst-1', '/home/cenk/project')
    invoke.mockResolvedValue({ id: 'inst-1' })

    openCwdLeaf()
    const spec = usePalette().stack.value[0]

    await spec?.onCommit([], 'src/components')
    await nextTick()

    expect(invoke).toHaveBeenCalledWith(TauriCommand.InstanceRestart, {
      instanceId: 'inst-1',
      cwd: '/home/cenk/project/src/components'
    })
  })

  it('successful commit pushes the (expanded) cwd onto history', async() => {
    useActiveInstance().set('inst-1')
    invoke.mockResolvedValue({ id: 'inst-1' })

    openCwdLeaf()
    const spec = usePalette().stack.value[0]

    await spec?.onCommit([], '/var/log')
    await nextTick()

    expect(useCwdHistory().history.value[0]).toBe('/var/log')
  })

  it('failed restart does not push to history', async() => {
    useActiveInstance().set('inst-1')
    invoke.mockRejectedValue(new Error('cwd not a directory'))

    openCwdLeaf()
    const spec = usePalette().stack.value[0]

    await spec?.onCommit([], '/nonexistent')
    await nextTick()

    expect(useCwdHistory().history.value).toEqual([])
  })
})
