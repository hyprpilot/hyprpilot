import { beforeEach, describe, expect, it, vi } from 'vitest'

import { useAdapter } from '@composables/useAdapter'

const invoke = vi.fn()

vi.mock('@ipc', () => ({
  invoke: (command: string, args?: Record<string, unknown>) => invoke(command, args),
  listen: vi.fn()
}))

beforeEach(() => {
  invoke.mockReset()
})

describe('useAdapter', () => {
  it('submit() invokes acp_submit with camel-cased args', async () => {
    invoke.mockResolvedValue({ accepted: true, agent_id: 'a' })
    const { submit } = useAdapter()

    await submit({ text: 'hi', profileId: 'strict' })

    expect(invoke).toHaveBeenCalledWith('acp_submit', {
      text: 'hi',
      agentId: undefined,
      profileId: 'strict'
    })
  })

  it('cancel() invokes acp_cancel with agentId', async () => {
    invoke.mockResolvedValue({ cancelled: true })
    const { cancel } = useAdapter()

    await cancel('a')

    expect(invoke).toHaveBeenCalledWith('acp_cancel', { agentId: 'a' })
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
