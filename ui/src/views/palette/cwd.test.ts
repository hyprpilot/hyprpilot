import { beforeEach, describe, expect, it, vi } from 'vitest'

import { pickCwd } from './cwd'
import { useActiveInstance, __resetUseProfilesForTests } from '@composables'
import { TauriCommand } from '@ipc'

const { invoke, openDialog, isRemoteHost } = vi.hoisted(() => ({
  invoke: vi.fn(),
  openDialog: vi.fn(),
  isRemoteHost: vi.fn(() => false)
}))

vi.mock('@ipc', async() => ({
  ...(await vi.importActual<object>('@ipc')),
  invoke: (command: string, args?: Record<string, unknown>) => invoke(command, args),
  listen: vi.fn()
}))

vi.mock('@ipc/remote-bridge', () => ({
  isRemoteHost: () => isRemoteHost()
}))

vi.mock('@tauri-apps/plugin-dialog', () => ({
  open: (opts?: Record<string, unknown>) => openDialog(opts)
}))

beforeEach(() => {
  invoke.mockReset()
  openDialog.mockReset()
  isRemoteHost.mockReturnValue(false)
  __resetUseProfilesForTests()
  useActiveInstance().id.value = undefined
})

describe('pickCwd', () => {
  it('pops the folder picker and commits the chosen path via instance_restart', async() => {
    useActiveInstance().set('inst-1')
    openDialog.mockResolvedValue('/srv/projects/foo')
    invoke.mockImplementation((command: string) => {
      if (command === TauriCommand.ProfilesList) {
        return Promise.resolve({ profiles: [] })
      }

      if (command === TauriCommand.ProfileGet) {
        return Promise.resolve(null)
      }

      return Promise.resolve({ id: 'inst-1' })
    })

    await pickCwd()

    expect(openDialog).toHaveBeenCalledWith(expect.objectContaining({ directory: true, multiple: false }))
    expect(invoke).toHaveBeenCalledWith(TauriCommand.InstanceRestart, {
      instanceId: 'inst-1',
      cwd: '/srv/projects/foo',
      ensure: true,
      agentId: undefined,
      profileId: undefined
    })
  })

  it('cancelled picker (null return) does NOT invoke instance_restart', async() => {
    useActiveInstance().set('inst-1')
    openDialog.mockResolvedValue(null)

    await pickCwd()

    expect(invoke).not.toHaveBeenCalledWith(TauriCommand.InstanceRestart, expect.any(Object))
  })

  it('cancelled picker (empty string) does NOT invoke instance_restart', async() => {
    useActiveInstance().set('inst-1')
    openDialog.mockResolvedValue('')

    await pickCwd()

    expect(invoke).not.toHaveBeenCalledWith(TauriCommand.InstanceRestart, expect.any(Object))
  })

  it('forwards ensure:true so the daemon prewarms when no active instance exists', async() => {
    openDialog.mockResolvedValue('/tmp/x')
    invoke.mockImplementation((command: string) => {
      if (command === TauriCommand.ProfilesList) {
        return Promise.resolve({ profiles: [] })
      }

      if (command === TauriCommand.ProfileGet) {
        return Promise.resolve(null)
      }

      return Promise.resolve({ id: 'inst-fresh' })
    })

    await pickCwd()

    expect(invoke).toHaveBeenCalledWith(TauriCommand.InstanceRestart, {
      instanceId: undefined,
      cwd: '/tmp/x',
      ensure: true,
      agentId: undefined,
      profileId: undefined
    })
  })

  it('on remote host, skips the picker AND skips instance_restart', async() => {
    isRemoteHost.mockReturnValue(true)
    useActiveInstance().set('inst-1')

    await pickCwd()

    expect(openDialog).not.toHaveBeenCalled()
    expect(invoke).not.toHaveBeenCalledWith(TauriCommand.InstanceRestart, expect.any(Object))
  })

  it('picker rejection (Tauri dialog plugin missing) does NOT invoke instance_restart', async() => {
    useActiveInstance().set('inst-1')
    openDialog.mockRejectedValue(new Error('dialog plugin not available'))

    await pickCwd()

    expect(invoke).not.toHaveBeenCalledWith(TauriCommand.InstanceRestart, expect.any(Object))
  })
})
