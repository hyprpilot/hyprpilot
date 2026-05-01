import { beforeEach, describe, expect, it, vi } from 'vitest'

import { TauriCommand } from '@ipc'

const { invoke } = vi.hoisted(() => ({
  invoke: vi.fn()
}))

vi.mock('@ipc/bridge', async () => ({
  ...(await vi.importActual<object>('@ipc/bridge')),
  invoke: (command: string, args?: Record<string, unknown>) => invoke(command, args)
}))

import { __resetPaletteStackForTests, PaletteMode, usePalette } from '@composables'
import { useActiveInstance } from '@composables'
import { __resetAllSessionInfoForTests } from '@composables'
import { clearToasts, useToasts } from '@composables'

import { openModesLeaf } from './modes'

vi.mock('@composables', async (importOriginal) => ({
  ...(await importOriginal<typeof import('@composables')>()),
  useProfiles: () => ({
    profiles: { value: [] },
    selected: { value: undefined },
    refresh: vi.fn(),
    select: vi.fn(),
    loading: { value: false },
    lastErr: { value: undefined }
  })
}))

const INSTANCE_ID = 'inst-A'

beforeEach(() => {
  __resetPaletteStackForTests()
  __resetAllSessionInfoForTests()
  clearToasts()
  invoke.mockReset()
  useActiveInstance().id.value = undefined
})

describe('openModesLeaf', () => {
  it('shows the no-instance row when no active instance is set', async () => {
    await openModesLeaf()

    const top = usePalette().stack.value.at(-1)
    expect(top?.title).toBe('modes')
    expect(top?.entries).toHaveLength(1)
    expect(top?.entries[0]?.name).toBe('no active instance')
  })

  it('shows the no-options row when the instance has no advertised modes', async () => {
    useActiveInstance().id.value = INSTANCE_ID
    invoke.mockResolvedValueOnce({
      cwd: '/tmp',
      availableModes: [],
      availableModels: []
    })

    await openModesLeaf()

    const top = usePalette().stack.value.at(-1)
    expect(top?.entries).toHaveLength(1)
    expect(top?.entries[0]?.name).toBe('no modes available')
  })

  it('lists every advertised mode from instance_meta and preseeds the active selection', async () => {
    useActiveInstance().id.value = INSTANCE_ID
    invoke.mockResolvedValueOnce({
      cwd: '/tmp',
      currentModeId: 'plan',
      availableModes: [
        { id: 'plan', name: 'Plan' },
        { id: 'edit', name: 'Edit' }
      ],
      availableModels: []
    })

    await openModesLeaf()

    expect(invoke).toHaveBeenCalledWith(TauriCommand.InstanceMeta, { instanceId: INSTANCE_ID })

    const top = usePalette().stack.value.at(-1)
    expect(top?.mode).toBe(PaletteMode.Select)
    expect(top?.entries.map((e) => e.id)).toEqual(['plan', 'edit'])
    expect(top?.preseedActive?.[0]?.id).toBe('plan')
  })

  it('fires modes_set with the picked id and surfaces success toast', async () => {
    useActiveInstance().id.value = INSTANCE_ID
    invoke.mockResolvedValueOnce({
      cwd: '/tmp',
      availableModes: [
        { id: 'plan', name: 'Plan' },
        { id: 'edit', name: 'Edit' }
      ],
      availableModels: []
    })
    invoke.mockResolvedValueOnce({})

    await openModesLeaf()
    const top = usePalette().stack.value.at(-1)
    await top?.onCommit([{ id: 'edit', name: 'Edit' }])

    expect(invoke).toHaveBeenCalledWith(TauriCommand.ModesSet, {
      instanceId: INSTANCE_ID,
      modeId: 'edit'
    })
    const toasts = useToasts().entries.value
    expect(toasts.at(-1)?.body).toBe('mode → Edit')
  })

  it('surfaces the RPC error verbatim via toast on failure', async () => {
    useActiveInstance().id.value = INSTANCE_ID
    invoke.mockResolvedValueOnce({
      cwd: '/tmp',
      availableModes: [{ id: 'plan', name: 'Plan' }],
      availableModels: []
    })
    invoke.mockRejectedValueOnce('modes/set not implemented — ref K-251')

    await openModesLeaf()
    const top = usePalette().stack.value.at(-1)
    await top?.onCommit([{ id: 'plan', name: 'Plan' }])

    const toasts = useToasts().entries.value
    const head = toasts.at(-1)?.body
    expect(typeof head === 'string' && head.includes('not implemented')).toBe(true)
  })

  it('no-ops when the user commits with an empty pick set', async () => {
    useActiveInstance().id.value = INSTANCE_ID
    invoke.mockResolvedValueOnce({
      cwd: '/tmp',
      availableModes: [{ id: 'plan', name: 'Plan' }],
      availableModels: []
    })

    await openModesLeaf()
    const top = usePalette().stack.value.at(-1)
    await top?.onCommit([])

    // Only the instance_meta call — no modes_set.
    expect(invoke).toHaveBeenCalledTimes(1)
    expect(invoke).toHaveBeenCalledWith(TauriCommand.InstanceMeta, { instanceId: INSTANCE_ID })
  })

  it('shows an error row when instance_meta fails', async () => {
    useActiveInstance().id.value = INSTANCE_ID
    invoke.mockRejectedValueOnce('actor closed')

    await openModesLeaf()

    const top = usePalette().stack.value.at(-1)
    expect(top?.entries).toHaveLength(1)
    expect(top?.entries[0]?.name).toBe('modes fetch failed')
  })
})
