import { beforeEach, describe, expect, it, vi } from 'vitest'

import { TauriCommand } from '@ipc'

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }))

vi.mock('@ipc/bridge', async () => ({
  ...(await vi.importActual<object>('@ipc/bridge')),
  invoke: (command: string, args?: Record<string, unknown>) => invoke(command, args),
  listen: vi.fn()
}))

import { useAdapter } from '@composables'

beforeEach(() => {
  invoke.mockReset()
})

describe('useAdapter', () => {
  it('submit() invokes session_submit with camel-cased args', async () => {
    invoke.mockResolvedValue({ accepted: true, agentId: 'a' })
    const { submit } = useAdapter()

    await submit({ text: 'hi', instanceId: 'i-1', profileId: 'strict' })

    expect(invoke).toHaveBeenCalledWith(TauriCommand.SessionSubmit, {
      text: 'hi',
      instanceId: 'i-1',
      agentId: undefined,
      profileId: 'strict',
      attachments: []
    })
  })

  it('submit() forwards attachments when provided', async () => {
    invoke.mockResolvedValue({ accepted: true, agentId: 'a' })
    const { submit } = useAdapter()
    const attachments = [{ slug: 'debug', path: '/skills/debug.md', body: 'body', title: 'Debug' }]

    await submit({ text: 'hi', profileId: 'strict', attachments })

    expect(invoke).toHaveBeenCalledWith(TauriCommand.SessionSubmit, {
      text: 'hi',
      instanceId: undefined,
      agentId: undefined,
      profileId: 'strict',
      attachments
    })
  })

  it('cancel() invokes session_cancel with instanceId + agentId', async () => {
    invoke.mockResolvedValue({ cancelled: true })
    const { cancel } = useAdapter()

    await cancel({ agentId: 'a' })

    expect(invoke).toHaveBeenCalledWith(TauriCommand.SessionCancel, {
      instanceId: undefined,
      agentId: 'a'
    })
  })

  it('agentsList() unwraps { agents } into an array', async () => {
    invoke.mockResolvedValue({ agents: [{ id: 'a', provider: 'acp-claude-code', isDefault: true }] })
    const { agentsList } = useAdapter()

    const agents = await agentsList()
    expect(agents).toHaveLength(1)
    expect(agents[0]?.id).toBe('a')
  })

  it('profilesList() unwraps { profiles } into an array', async () => {
    invoke.mockResolvedValue({ profiles: [{ id: 'p', agent: 'a', isDefault: true }] })
    const { profilesList } = useAdapter()

    const profiles = await profilesList()
    expect(profiles).toHaveLength(1)
    expect(profiles[0]?.id).toBe('p')
  })
})
