import { beforeEach, describe, expect, it, vi } from 'vitest'

import { TauriCommand } from '@ipc'

const { invokeMock } = vi.hoisted(() => ({ invokeMock: vi.fn() }))

// Mock @ipc/bridge directly: @ipc is a barrel that re-exports `invoke`
// from bridge.ts; the re-export binds the reference at module-evaluation
// time, before vi.mock('@ipc', ...) can replace it. Targeting bridge.ts
// is the only way the consumer's `import { invoke } from '@ipc'` picks
// up our mock.
vi.mock('@ipc/bridge', async () => ({
  ...(await vi.importActual<object>('@ipc/bridge')),
  invoke: (...args: unknown[]) => invokeMock(...args)
}))

import { __resetHomeDirForTests, loadHomeDir, useHomeDir } from './use-home-dir'

beforeEach(() => {
  invokeMock.mockReset()
  __resetHomeDirForTests()
})

describe('useHomeDir', () => {
  it('starts undefined before loadHomeDir resolves', () => {
    expect(useHomeDir().homeDir.value).toBeUndefined()
  })

  it('caches the resolved $HOME from get_home_dir', async () => {
    invokeMock.mockResolvedValueOnce('/home/cenk')
    await loadHomeDir()
    expect(useHomeDir().homeDir.value).toBe('/home/cenk')
    expect(invokeMock).toHaveBeenCalledWith(TauriCommand.GetHomeDir)
  })

  it('soft-fails to undefined when invoke throws (no Tauri host)', async () => {
    invokeMock.mockRejectedValueOnce(new Error('host missing'))
    await loadHomeDir()
    expect(useHomeDir().homeDir.value).toBeUndefined()
  })
})
