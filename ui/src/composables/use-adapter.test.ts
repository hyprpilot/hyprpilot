import { beforeEach, describe, expect, it, vi } from 'vitest'

import { TauriCommand } from '@ipc'

import { useAdapter } from '@composables/use-adapter'

const invoke = vi.fn()

vi.mock('@ipc', async () => {
  const actual = await vi.importActual<typeof import('@ipc')>('@ipc')
  return {
    ...actual,
    invoke: (command: string, args?: Record<string, unknown>) => invoke(command, args),
    listen: vi.fn()
  }
})

beforeEach(() => {
  invoke.mockReset()
})

describe('useAdapter', () => {
  it('submit() invokes session_submit with camel-cased args', async () => {
    invoke.mockResolvedValue({ accepted: true, agent_id: 'a' })
    const { submit } = useAdapter()

    await submit({ text: 'hi', instanceId: 'i-1', profileId: 'strict' })

    expect(invoke).toHaveBeenCalledWith(TauriCommand.SessionSubmit, {
      text: 'hi',
      instanceId: 'i-1',
      agentId: undefined,
      profileId: 'strict'
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
    invoke.mockResolvedValue({ agents: [{ id: 'a', provider: 'acp-claude-code', is_default: true }] })
    const { agentsList } = useAdapter()

    const agents = await agentsList()
    expect(agents).toHaveLength(1)
    expect(agents[0]?.id).toBe('a')
  })

  it('profilesList() unwraps { profiles } into an array', async () => {
    invoke.mockResolvedValue({ profiles: [{ id: 'p', agent: 'a', has_prompt: false, is_default: true }] })
    const { profilesList } = useAdapter()

    const profiles = await profilesList()
    expect(profiles).toHaveLength(1)
    expect(profiles[0]?.id).toBe('p')
  })
})
