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

/**
 * Default mock: `paths_resolve` returns its `cwdBase`-joined value
 * (mirrors daemon-side `tools::path::resolve_absolute` behaviour
 * for the cases tested below). `instance_restart` resolves with a
 * stub.
 */
function mockResolveAndRestart(): void {
  invoke.mockImplementation((command: string, args?: Record<string, unknown>) => {
    if (command === TauriCommand.PathsResolve) {
      const raw = (args?.raw as string).trim()
      const cwdBase = args?.cwdBase as string | undefined

      if (!raw) {
        return Promise.resolve(null)
      }

      if (raw.startsWith('/')) {
        return Promise.resolve(raw)
      }

      if (raw === '~' || raw.startsWith('~/')) {
        const home = useHomeDir().homeDir.value

        if (!home) {
          return Promise.resolve(null)
        }

        return Promise.resolve(raw === '~' ? home : `${home}${raw.slice(1)}`)
      }

      if (!cwdBase) {
        return Promise.resolve(null)
      }
      const base = cwdBase.replace(/\/$/, '')

      if (raw === '.') {
        return Promise.resolve(base)
      }
      const stripped = raw.startsWith('./') ? raw.slice(2) : raw

      return Promise.resolve(`${base}/${stripped}`)
    }

    if (command === TauriCommand.InstanceRestart) {
      return Promise.resolve({ id: 'inst-1' })
    }

    return Promise.resolve(undefined)
  })
}

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
    expect(stack.value[0]?.entries).toHaveLength(0)
  })

  it('lists recent history entries when history is non-empty', () => {
    const { push } = useCwdHistory()

    push('/tmp/a')
    push('/tmp/b')

    openCwdLeaf()
    const entries = usePalette().stack.value[0]?.entries ?? []
    const ids = entries.map((e: PaletteEntry) => e.id)

    expect(ids).toEqual(['cwd-recent:/tmp/b', 'cwd-recent:/tmp/a'])
  })

  it('committing a recent row invokes instance_restart with that path', async() => {
    useActiveInstance().set('inst-1')
    const { push } = useCwdHistory()

    push('/home/cenk/dev')
    mockResolveAndRestart()

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
    mockResolveAndRestart()

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
    mockResolveAndRestart()

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
    mockResolveAndRestart()

    openCwdLeaf()
    const spec = usePalette().stack.value[0]

    await spec?.onCommit([], 'relative/path')

    // paths_resolve fires + returns null; instance_restart never reached.
    expect(invoke).toHaveBeenCalledWith(TauriCommand.PathsResolve, expect.any(Object))
    expect(invoke).not.toHaveBeenCalledWith(TauriCommand.InstanceRestart, expect.any(Object))
  })

  it('rejects empty input without invoking restart', async() => {
    useActiveInstance().set('inst-1')
    mockResolveAndRestart()

    openCwdLeaf()
    const spec = usePalette().stack.value[0]

    await spec?.onCommit([], '   ')

    expect(invoke).not.toHaveBeenCalledWith(TauriCommand.InstanceRestart, expect.any(Object))
  })

  it('refuses to commit when no active instance exists', async() => {
    mockResolveAndRestart()

    openCwdLeaf()
    const spec = usePalette().stack.value[0]

    await spec?.onCommit([], '/tmp/x')

    expect(invoke).not.toHaveBeenCalledWith(TauriCommand.InstanceRestart, expect.any(Object))
  })

  it('resolves a relative path against the active instance cwd before invoking restart', async() => {
    const { setInstanceCwd } = await import('@composables')

    useActiveInstance().set('inst-1')
    setInstanceCwd('inst-1', '/home/cenk/project')
    mockResolveAndRestart()

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
    mockResolveAndRestart()

    openCwdLeaf()
    const spec = usePalette().stack.value[0]

    await spec?.onCommit([], '/var/log')
    await nextTick()

    expect(useCwdHistory().history.value[0]).toBe('/var/log')
  })

  it('failed restart does not push to history', async() => {
    useActiveInstance().set('inst-1')
    invoke.mockImplementation((command: string) => {
      if (command === TauriCommand.PathsResolve) {
        return Promise.resolve('/nonexistent')
      }

      return Promise.reject(new Error('cwd not a directory'))
    })

    openCwdLeaf()
    const spec = usePalette().stack.value[0]

    await spec?.onCommit([], '/nonexistent')
    await nextTick()

    expect(useCwdHistory().history.value).toEqual([])
  })
})
