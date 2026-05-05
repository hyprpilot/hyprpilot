import { beforeEach, describe, expect, it, vi } from 'vitest'

import { useActiveInstance, evictPermission, pushPermissionRequest, resetPermissions, usePermissions } from '@composables'
import { TauriCommand } from '@ipc'

function fmt() {
  return {
    title: 'bash',
    stats: [],
    fields: []
  }
}

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }))

vi.mock('@ipc/bridge', async() => ({
  ...(await vi.importActual<object>('@ipc/bridge')),
  invoke: (command: string, args?: Record<string, unknown>) => invoke(command, args),
  listen: vi.fn()
}))

beforeEach(() => {
  invoke.mockReset()
  resetPermissions('A')
  resetPermissions('B')
  useActiveInstance().id.value = undefined
})

function raw(requestId: string, overrides: Partial<{ tool: string; args: string; kind: string }> = {}) {
  return {
    agentId: 'agent-A',
    requestId,
    tool: overrides.tool ?? 'bash',
    kind: overrides.kind ?? 'execute',
    args: overrides.args ?? 'echo hi',
    options: [
      {
        optionId: 'allow-once-id',
        name: 'Allow once',
        kind: 'allow_once'
      },
      {
        optionId: 'allow-always-id',
        name: 'Allow always',
        kind: 'allow_always'
      },
      {
        optionId: 'reject-once-id',
        name: 'Reject once',
        kind: 'reject_once'
      }
    ],
    formatted: fmt()
  }
}

describe('usePermissions', () => {
  it('accumulates row-queue prompts per instance; A never sees B prompts', () => {
    pushPermissionRequest('A', 's-a', raw('req-a1'))
    pushPermissionRequest('A', 's-a', raw('req-a2'))
    pushPermissionRequest('B', 's-b', raw('req-b1'))

    const a = usePermissions('A').rowQueue.value
    const b = usePermissions('B').rowQueue.value

    expect(a.map((v) => v.request.requestId)).toEqual(['req-a1', 'req-a2'])
    expect(b.map((v) => v.request.requestId)).toEqual(['req-b1'])
  })

  it('emits row queue oldest-first by createdAt', () => {
    pushPermissionRequest('A', 's-a', raw('first'))
    pushPermissionRequest('A', 's-a', raw('second'))
    pushPermissionRequest('A', 's-a', raw('third'))

    const queue = usePermissions('A').rowQueue.value

    expect(queue.map((v) => v.request.requestId)).toEqual(['first', 'second', 'third'])
  })

  it('marks every prompt after the first as queued', () => {
    pushPermissionRequest('A', 's-a', raw('req-1'))
    pushPermissionRequest('A', 's-a', raw('req-2'))

    const queue = usePermissions('A').rowQueue.value

    expect(queue[0]?.queued).toBe(false)
    expect(queue[1]?.queued).toBe(true)
  })

  it('promotes the next-oldest entry to active after the current one is evicted', () => {
    pushPermissionRequest('A', 's-a', raw('req-1'))
    pushPermissionRequest('A', 's-a', raw('req-2'))
    pushPermissionRequest('A', 's-a', raw('req-3'))

    const before = usePermissions('A').rowQueue.value

    expect(before.map((v) => [v.request.requestId, v.queued])).toEqual([
      ['req-1', false],
      ['req-2', true],
      ['req-3', true]
    ])

    evictPermission('A', 'req-1')

    const after = usePermissions('A').rowQueue.value

    expect(after.map((v) => [v.request.requestId, v.queued])).toEqual([
      ['req-2', false],
      ['req-3', true]
    ])
  })

  it('routes plan-exit prompts to the modal queue', () => {
    pushPermissionRequest('A', 's-a', {
      agentId: 'agent-A',
      requestId: 'plan-1',
      tool: 'EditFile',
      kind: 'edit',
      args: '',
      rawInput: { plan: '# Plan\n\n- step 1\n- step 2' },
      options: raw('plan-1').options,
      formatted: fmt()
    })

    const { rowQueue, modalQueue } = usePermissions('A')

    expect(rowQueue.value).toHaveLength(0)
    expect(modalQueue.value).toHaveLength(1)
    expect(modalQueue.value[0]?.request.requestId).toBe('plan-1')
  })

  it('respond() invokes permission_reply with the captain-supplied optionId and evicts on success', async() => {
    pushPermissionRequest('A', 's-a', raw('req-1'))
    invoke.mockResolvedValue(undefined)

    await usePermissions('A').respond('req-1', 'allow-once-id')

    expect(invoke).toHaveBeenCalledWith(TauriCommand.PermissionReply, {
      sessionId: 's-a',
      requestId: 'req-1',
      optionId: 'allow-once-id'
    })
    expect(usePermissions('A').rowQueue.value).toHaveLength(0)
  })

  it('respond() with the allow_always option id forwards verbatim — kind-driven trust write happens daemon-side', async() => {
    pushPermissionRequest('A', 's-a', raw('req-1'))
    invoke.mockResolvedValue(undefined)

    await usePermissions('A').respond('req-1', 'allow-always-id')

    expect(invoke).toHaveBeenCalledWith(TauriCommand.PermissionReply, {
      sessionId: 's-a',
      requestId: 'req-1',
      optionId: 'allow-always-id'
    })
  })

  it('respond() with the reject option id evicts on success', async() => {
    pushPermissionRequest('A', 's-a', raw('req-1'))
    invoke.mockResolvedValue(undefined)

    await usePermissions('A').respond('req-1', 'reject-once-id')

    expect(invoke).toHaveBeenCalledWith(TauriCommand.PermissionReply, {
      sessionId: 's-a',
      requestId: 'req-1',
      optionId: 'reject-once-id'
    })
    expect(usePermissions('A').rowQueue.value).toHaveLength(0)
  })

  it('respond() throws when invoke rejects and leaves the pending entry in place', async() => {
    pushPermissionRequest('A', 's-a', raw('req-1'))
    invoke.mockRejectedValue(new Error('permission_reply not implemented'))

    await expect(usePermissions('A').respond('req-1', 'allow-once-id')).rejects.toThrow('permission_reply not implemented')
    expect(usePermissions('A').rowQueue.value).toHaveLength(1)
  })

  it('throws when the requestId has no pending entry', async() => {
    await expect(usePermissions('A').respond('nonexistent', 'allow-once-id')).rejects.toThrow('no pending permission request nonexistent')
    expect(invoke).not.toHaveBeenCalled()
  })

  it('throws when no instance is active and no explicit id is passed', async() => {
    pushPermissionRequest('A', 's-a', raw('req-1'))
    await expect(usePermissions().respond('req-1', 'allow-once-id')).rejects.toThrow('no active instance')
  })

  it('resolves through useActiveInstance when no id is passed', () => {
    useActiveInstance().set('A')
    pushPermissionRequest('A', 's-a', raw('req-a'))
    pushPermissionRequest('B', 's-b', raw('req-b'))

    const implicit = usePermissions().rowQueue.value

    expect(implicit.map((v) => v.request.requestId)).toEqual(['req-a'])
  })

  it('evictPermission removes the specific entry without disturbing siblings', () => {
    pushPermissionRequest('A', 's-a', raw('keep'))
    pushPermissionRequest('A', 's-a', raw('drop'))

    evictPermission('A', 'drop')

    const queue = usePermissions('A').rowQueue.value

    expect(queue.map((v) => v.request.requestId)).toEqual(['keep'])
  })

  it('resetPermissions clears the whole slot for an instance', () => {
    pushPermissionRequest('A', 's-a', raw('req-1'))
    pushPermissionRequest('A', 's-a', raw('req-2'))

    resetPermissions('A')

    expect(usePermissions('A').rowQueue.value).toEqual([])
  })
})
