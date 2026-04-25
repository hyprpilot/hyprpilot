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

import { openModelsLeaf } from './palette-models'

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

describe('openModelsLeaf', () => {
  it('shows the no-instance row when no active instance is set', () => {
    openModelsLeaf()

    const top = usePalette().stack.value.at(-1)
    expect(top?.title).toBe('models')
    expect(top?.entries).toHaveLength(1)
    expect(top?.entries[0]?.name).toBe('no active instance')
  })

  it('shows the no-options row when the instance has no advertised models', () => {
    useActiveInstance().id.value = INSTANCE_ID
    openModelsLeaf()

    const top = usePalette().stack.value.at(-1)
    expect(top?.entries).toHaveLength(1)
    expect(top?.entries[0]?.name).toBe('no models available')
  })

  it('lists every advertised model and preseeds the active selection', () => {
    useActiveInstance().id.value = INSTANCE_ID
    pushSessionInfoUpdate(INSTANCE_ID, {
      currentModelId: 'sonnet',
      availableModels: [
        { id: 'sonnet', name: 'Claude Sonnet 4.5' },
        { id: 'opus', name: 'Claude Opus 4.5' }
      ]
    })

    openModelsLeaf()

    const top = usePalette().stack.value.at(-1)
    expect(top?.mode).toBe(PaletteMode.Select)
    expect(top?.entries.map((e) => e.id)).toEqual(['sonnet', 'opus'])
    expect(top?.preseedActive?.[0]?.id).toBe('sonnet')
  })

  it('fires models_set with the picked id and surfaces success toast', async () => {
    useActiveInstance().id.value = INSTANCE_ID
    pushSessionInfoUpdate(INSTANCE_ID, {
      availableModels: [
        { id: 'sonnet', name: 'Claude Sonnet 4.5' },
        { id: 'opus', name: 'Claude Opus 4.5' }
      ]
    })
    invoke.mockResolvedValueOnce({})

    openModelsLeaf()
    const top = usePalette().stack.value.at(-1)
    await top?.onCommit([{ id: 'opus', name: 'Claude Opus 4.5' }])

    expect(invoke).toHaveBeenCalledWith(TauriCommand.ModelsSet, {
      instanceId: INSTANCE_ID,
      modelId: 'opus'
    })
    const toasts = useToasts().entries.value
    expect(toasts.at(-1)?.message).toBe('model → Claude Opus 4.5')
  })

  it('surfaces the RPC error verbatim via toast on failure', async () => {
    useActiveInstance().id.value = INSTANCE_ID
    pushSessionInfoUpdate(INSTANCE_ID, {
      availableModels: [{ id: 'sonnet', name: 'Sonnet' }]
    })
    invoke.mockRejectedValueOnce('models/set not implemented — ref K-251')

    openModelsLeaf()
    const top = usePalette().stack.value.at(-1)
    await top?.onCommit([{ id: 'sonnet', name: 'Sonnet' }])

    const toasts = useToasts().entries.value
    expect(toasts.at(-1)?.message).toContain('not implemented')
  })

  it('no-ops when the user commits with an empty pick set', async () => {
    useActiveInstance().id.value = INSTANCE_ID
    pushSessionInfoUpdate(INSTANCE_ID, {
      availableModels: [{ id: 'sonnet', name: 'Sonnet' }]
    })

    openModelsLeaf()
    const top = usePalette().stack.value.at(-1)
    await top?.onCommit([])

    expect(invoke).not.toHaveBeenCalled()
  })
})
