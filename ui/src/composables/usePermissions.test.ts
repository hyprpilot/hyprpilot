import { beforeEach, describe, expect, it, vi } from 'vitest'

import { TauriCommand } from '@ipc'

import { useActiveInstance } from '@composables/useActiveInstance'
import {
  evictPermission,
  pushPermissionRequest,
  resetPermissions,
  usePermissions
} from '@composables/usePermissions'

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
  resetPermissions('A')
  resetPermissions('B')
  useActiveInstance().id.value = undefined
})

function raw(requestId: string, overrides: Partial<{ tool: string; args: string; kind: string }> = {}) {
  return {
    request_id: requestId,
    tool: overrides.tool ?? 'bash',
    kind: overrides.kind ?? 'bash',
    args: overrides.args ?? 'echo hi',
    options: [{ option_id: 'allow', name: 'Allow', kind: 'y' }]
  }
}

describe('usePermissions', () => {
  it('accumulates prompts per instance; A never sees B prompts', () => {
    pushPermissionRequest('A', 's-a', raw('req-a1'))
    pushPermissionRequest('A', 's-a', raw('req-a2'))
    pushPermissionRequest('B', 's-b', raw('req-b1'))

    const a = usePermissions('A').pending.value
    const b = usePermissions('B').pending.value

    expect(a.map((p) => p.requestId)).toEqual(['req-a1', 'req-a2'])
    expect(b.map((p) => p.requestId)).toEqual(['req-b1'])
  })

  it('emits pending oldest-first by createdAt', () => {
    pushPermissionRequest('A', 's-a', raw('first'))
    pushPermissionRequest('A', 's-a', raw('second'))
    pushPermissionRequest('A', 's-a', raw('third'))

    const pending = usePermissions('A').pending.value
    expect(pending.map((p) => p.requestId)).toEqual(['first', 'second', 'third'])
  })

  it('marks every prompt after the first as queued', () => {
    pushPermissionRequest('A', 's-a', raw('req-1'))
    pushPermissionRequest('A', 's-a', raw('req-2'))

    const pending = usePermissions('A').pending.value
    expect(pending[0]?.queued).toBe(false)
    expect(pending[1]?.queued).toBe(true)
  })

  it('promotes the next-oldest entry to active after the current one is evicted', () => {
    pushPermissionRequest('A', 's-a', raw('req-1'))
    pushPermissionRequest('A', 's-a', raw('req-2'))
    pushPermissionRequest('A', 's-a', raw('req-3'))

    const before = usePermissions('A').pending.value
    expect(before.map((p) => [p.requestId, p.queued])).toEqual([
      ['req-1', false],
      ['req-2', true],
      ['req-3', true]
    ])

    evictPermission('A', 'req-1')

    const after = usePermissions('A').pending.value
    expect(after.map((p) => [p.requestId, p.queued])).toEqual([
      ['req-2', false],
      ['req-3', true]
    ])
  })

  it('allow() invokes permission_reply with the entry session + request + optionId=allow', async () => {
    pushPermissionRequest('A', 's-a', raw('req-1'))
    invoke.mockResolvedValue(undefined)

    await usePermissions('A').allow('req-1')

    expect(invoke).toHaveBeenCalledWith(TauriCommand.PermissionReply, {
      sessionId: 's-a',
      requestId: 'req-1',
      optionId: 'allow'
    })
    expect(usePermissions('A').pending.value).toHaveLength(0)
  })

  it('deny() invokes permission_reply with optionId=deny and evicts on success', async () => {
    pushPermissionRequest('A', 's-a', raw('req-1'))
    invoke.mockResolvedValue(undefined)

    await usePermissions('A').deny('req-1')

    expect(invoke).toHaveBeenCalledWith(TauriCommand.PermissionReply, {
      sessionId: 's-a',
      requestId: 'req-1',
      optionId: 'deny'
    })
    expect(usePermissions('A').pending.value).toHaveLength(0)
  })

  it('allow() throws when invoke rejects and leaves the pending entry in place', async () => {
    pushPermissionRequest('A', 's-a', raw('req-1'))
    invoke.mockRejectedValue(new Error('permission_reply not implemented'))

    await expect(usePermissions('A').allow('req-1')).rejects.toThrow('permission_reply not implemented')
    expect(usePermissions('A').pending.value).toHaveLength(1)
  })

  it('throws when the requestId has no pending entry', async () => {
    await expect(usePermissions('A').allow('nonexistent')).rejects.toThrow('no pending permission request nonexistent')
    expect(invoke).not.toHaveBeenCalled()
  })

  it('throws when no instance is active and no explicit id is passed', async () => {
    pushPermissionRequest('A', 's-a', raw('req-1'))
    await expect(usePermissions().allow('req-1')).rejects.toThrow('no active instance')
  })

  it('resolves through useActiveInstance when no id is passed', () => {
    useActiveInstance().set('A')
    pushPermissionRequest('A', 's-a', raw('req-a'))
    pushPermissionRequest('B', 's-b', raw('req-b'))

    const implicit = usePermissions().pending.value
    expect(implicit.map((p) => p.requestId)).toEqual(['req-a'])
  })

  it('evictPermission removes the specific entry without disturbing siblings', () => {
    pushPermissionRequest('A', 's-a', raw('keep'))
    pushPermissionRequest('A', 's-a', raw('drop'))

    evictPermission('A', 'drop')

    const pending = usePermissions('A').pending.value
    expect(pending.map((p) => p.requestId)).toEqual(['keep'])
  })

  it('resetPermissions clears the whole slot for an instance', () => {
    pushPermissionRequest('A', 's-a', raw('req-1'))
    pushPermissionRequest('A', 's-a', raw('req-2'))

    resetPermissions('A')

    expect(usePermissions('A').pending.value).toEqual([])
  })
})
