import { beforeEach, describe, expect, it, vi } from 'vitest'

import { openInstanceLeaf } from './instance'
import {
  __resetActiveInstanceForTests,
  __resetAllSessionInfoForTests,
  __resetPaletteStackForTests,
  __resetUseProfilesForTests,
  applyBootProfiles,
  clearToasts,
  useActiveInstance,
  usePalette,
  useToasts
} from '@composables'
import { TauriCommand } from '@ipc'

const { invokeMock, listenMock } = vi.hoisted(() => ({
  invokeMock: vi.fn(),
  listenMock: vi.fn()
}))

vi.mock('@ipc', async() => ({
  ...(await vi.importActual<object>('@ipc')),
  invoke: (...args: unknown[]) => invokeMock(...args),
  listen: (...args: unknown[]) => listenMock(...args)
}))

vi.mock('@lib', async() => ({
  ...(await vi.importActual<object>('@lib')),
  log: {
    trace: vi.fn(),
    debug: vi.fn(),
    info: vi.fn(),
    warn: vi.fn(),
    error: vi.fn()
  }
}))

function mockFocusedInstance(sessionId?: string): void {
  const resolvedSessionId = arguments.length === 0 ? 'source-session' : sessionId

  invokeMock.mockImplementation((command: string) => {
    if (command === TauriCommand.InstanceMeta) {
      return Promise.resolve({
        name: 'work',
        sessionId: resolvedSessionId,
        cwd: '/tmp/hyprpilot',
        availableModes: [],
        availableModels: [],
        mcpsCount: 0
      })
    }

    if (command === TauriCommand.InstanceSnapshotMeta) {
      return Promise.resolve({
        profileId: 'personal/claude',
        sessionId: resolvedSessionId,
        cwd: '/tmp/hyprpilot',
        availableModes: [],
        availableModels: [],
        mcpsCount: 0,
        usage: {
          used: 0,
          size: 0
        }
      })
    }

    if (command === TauriCommand.SessionFork) {
      return Promise.resolve({ instanceId: 'forked-instance' })
    }

    return Promise.resolve(undefined)
  })
}

beforeEach(() => {
  __resetActiveInstanceForTests()
  __resetAllSessionInfoForTests()
  __resetPaletteStackForTests()
  __resetUseProfilesForTests()
  clearToasts()
  invokeMock.mockReset()
  listenMock.mockReset()
  listenMock.mockResolvedValue(() => {})
})

describe('openInstanceLeaf', () => {
  it('shows fork for the focused instance', async() => {
    useActiveInstance().set('inst-source')
    mockFocusedInstance()

    await openInstanceLeaf()

    const entries = usePalette().stack.value[0]?.entries ?? []

    expect(entries.map((entry) => entry.id)).toContain('fork')
  })

  it('forks the focused session into a fresh instance id', async() => {
    const randomUUID = vi.spyOn(crypto, 'randomUUID').mockReturnValue('00000000-0000-4000-8000-000000000001')

    applyBootProfiles(
      [
        {
          id: 'personal/claude',
          agent: 'claude-code',
          isDefault: true
        }
      ],
      'personal/claude'
    )
    useActiveInstance().set('inst-source')
    mockFocusedInstance()

    await openInstanceLeaf()

    const spec = usePalette().stack.value[0]
    const fork = spec?.entries.find((entry) => entry.id === 'fork')

    expect(fork).toBeDefined()
    spec?.onCommit([fork!])

    const forkCall = invokeMock.mock.calls.find((call: unknown[]) => call[0] === TauriCommand.SessionFork)

    expect(forkCall).toBeDefined()
    expect(forkCall?.[1]).toEqual({
      agentId: 'claude-code',
      profileId: 'personal/claude',
      sessionId: 'source-session',
      instanceId: '00000000-0000-4000-8000-000000000001',
      cwd: '/tmp/hyprpilot'
    })
    randomUUID.mockRestore()
  })

  it('warns instead of calling session_fork when the focused instance has no session id', async() => {
    useActiveInstance().set('inst-source')
    mockFocusedInstance(undefined)

    await openInstanceLeaf()

    const spec = usePalette().stack.value[0]
    const fork = spec?.entries.find((entry) => entry.id === 'fork')

    expect(fork).toBeDefined()
    spec?.onCommit([fork!])

    const forkCall = invokeMock.mock.calls.find((call: unknown[]) => call[0] === TauriCommand.SessionFork)

    expect(forkCall).toBeUndefined()
    expect(useToasts().entries.value.at(-1)?.body).toBe('no live session to fork yet')
  })
})
