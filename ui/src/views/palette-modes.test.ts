import { beforeEach, describe, expect, it, vi } from 'vitest'

import { TauriCommand } from '@ipc'

const { invoke } = vi.hoisted(() => ({
  invoke: vi.fn()
}))

vi.mock('@ipc/bridge', async () => ({
  ...(await vi.importActual<object>('@ipc/bridge')),
  invoke: (command: string, args?: Record<string, unknown>) => invoke(command, args)
}))

import { __resetPaletteStackForTests, PaletteMode, usePalette } from '@composables/palette'
import { useActiveInstance } from '@composables/use-active-instance'
import { __resetAllSessionInfoForTests, pushSessionInfoUpdate } from '@composables/use-session-info'
import { clearToasts, useToasts } from '@composables/use-toasts'

import { openModesLeaf } from './palette-modes'

vi.mock('@composables/use-profiles', () => ({
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
  it('shows the no-instance row when no active instance is set', () => {
    openModesLeaf()

    const top = usePalette().stack.value.at(-1)
    expect(top?.title).toBe('modes')
    expect(top?.entries).toHaveLength(1)
    expect(top?.entries[0]?.name).toBe('no active instance')
  })

  it('shows the no-options row when the instance has no advertised modes', () => {
    useActiveInstance().id.value = INSTANCE_ID
    openModesLeaf()

    const top = usePalette().stack.value.at(-1)
    expect(top?.entries).toHaveLength(1)
    expect(top?.entries[0]?.name).toBe('no modes available')
  })

  it('lists every advertised mode and preseeds the active selection', () => {
    useActiveInstance().id.value = INSTANCE_ID
    pushSessionInfoUpdate(INSTANCE_ID, {
      currentModeId: 'plan',
      availableModes: [
        { id: 'plan', name: 'Plan' },
        { id: 'edit', name: 'Edit' }
      ]
    })

    openModesLeaf()

    const top = usePalette().stack.value.at(-1)
    expect(top?.mode).toBe(PaletteMode.Select)
    expect(top?.entries.map((e) => e.id)).toEqual(['plan', 'edit'])
    expect(top?.preseedActive?.[0]?.id).toBe('plan')
  })

  it('fires modes_set with the picked id and surfaces success toast', async () => {
    useActiveInstance().id.value = INSTANCE_ID
    pushSessionInfoUpdate(INSTANCE_ID, {
      availableModes: [
        { id: 'plan', name: 'Plan' },
        { id: 'edit', name: 'Edit' }
      ]
    })
    invoke.mockResolvedValueOnce({})

    openModesLeaf()
    const top = usePalette().stack.value.at(-1)
    await top?.onCommit([{ id: 'edit', name: 'Edit' }])

    expect(invoke).toHaveBeenCalledWith(TauriCommand.ModesSet, {
      instanceId: INSTANCE_ID,
      modeId: 'edit'
    })
    const toasts = useToasts().entries.value
    expect(toasts.at(-1)?.message).toBe('mode → Edit')
  })

  it('surfaces the RPC error verbatim via toast on failure', async () => {
    useActiveInstance().id.value = INSTANCE_ID
    pushSessionInfoUpdate(INSTANCE_ID, {
      availableModes: [{ id: 'plan', name: 'Plan' }]
    })
    invoke.mockRejectedValueOnce('modes/set not implemented — ref K-251')

    openModesLeaf()
    const top = usePalette().stack.value.at(-1)
    await top?.onCommit([{ id: 'plan', name: 'Plan' }])

    const toasts = useToasts().entries.value
    expect(toasts.at(-1)?.message).toContain('not implemented')
  })

  it('no-ops when the user commits with an empty pick set', async () => {
    useActiveInstance().id.value = INSTANCE_ID
    pushSessionInfoUpdate(INSTANCE_ID, {
      availableModes: [{ id: 'plan', name: 'Plan' }]
    })

    openModesLeaf()
    const top = usePalette().stack.value.at(-1)
    await top?.onCommit([])

    expect(invoke).not.toHaveBeenCalled()
  })
})
