import { beforeEach, describe, expect, it, vi } from 'vitest'

import { useActiveInstance } from '@composables/use-active-instance'
import {
  __resetAllSessionInfoForTests,
  pushSessionInfoUpdate,
  setSessionRestored,
  truncateCwd,
  useSessionInfo
} from '@composables/use-session-info'

const profilesRef = { value: [] as { id: string; agent: string; model?: string; isDefault: boolean }[] }
const selectedRef = { value: undefined as string | undefined }

vi.mock('@composables/use-profiles', () => ({
  useProfiles: () => ({
    profiles: profilesRef,
    selected: selectedRef,
    refresh: vi.fn(),
    select: vi.fn(),
    loading: { value: false },
    lastErr: { value: undefined }
  })
}))

beforeEach(() => {
  __resetAllSessionInfoForTests()
  profilesRef.value = []
  selectedRef.value = undefined
  useActiveInstance().id.value = undefined
})

describe('pushSessionInfoUpdate', () => {
  it('records the mode for the addressed instance', () => {
    pushSessionInfoUpdate('A', { currentModeId: 'plan' })

    useActiveInstance().id.value = 'A'
    expect(useSessionInfo().info.value.mode).toBe('plan')
  })

  it('falls back to mode field when currentModeId is absent', () => {
    pushSessionInfoUpdate('A', { mode: 'edit' })

    expect(useSessionInfo('A').info.value.mode).toBe('edit')
  })

  it('records the title when session_info_update arrives', () => {
    pushSessionInfoUpdate('A', { title: 'fix the build' })

    expect(useSessionInfo('A').info.value.title).toBe('fix the build')
  })

  it('clears the field when an explicit empty string arrives', () => {
    pushSessionInfoUpdate('A', { title: 'first' })
    pushSessionInfoUpdate('A', { title: '' })

    expect(useSessionInfo('A').info.value.title).toBe('')
  })

  it('preserves the field when an undefined value arrives', () => {
    pushSessionInfoUpdate('A', { title: 'first' })
    pushSessionInfoUpdate('A', { cwd: '/tmp' })

    expect(useSessionInfo('A').info.value.title).toBe('first')
  })

  it('isolates state between instances', () => {
    pushSessionInfoUpdate('A', { currentModeId: 'plan' })
    pushSessionInfoUpdate('B', { currentModeId: 'edit' })

    expect(useSessionInfo('A').info.value.mode).toBe('plan')
    expect(useSessionInfo('B').info.value.mode).toBe('edit')
  })
})

describe('useSessionInfo profile derivation', () => {
  it('derives model from the active profile when the instance has no override', () => {
    profilesRef.value = [{ id: 'ask', agent: 'claude-code', model: 'claude-sonnet-4-5', isDefault: true }]
    selectedRef.value = 'ask'

    expect(useSessionInfo('A').info.value.model).toBe('claude-sonnet-4-5')
  })

  it('prefers the instance model over the profile model when both exist', () => {
    profilesRef.value = [{ id: 'ask', agent: 'claude-code', model: 'claude-sonnet-4-5', isDefault: true }]
    selectedRef.value = 'ask'
    pushSessionInfoUpdate('A', { model: 'claude-opus-4-5' })

    expect(useSessionInfo('A').info.value.model).toBe('claude-opus-4-5')
  })

  it('always reports zero mcps and skills counts (live counts land in K-258 / K-268)', () => {
    profilesRef.value = [{ id: 'ask', agent: 'claude-code', isDefault: true }]
    selectedRef.value = 'ask'

    const info = useSessionInfo('A').info.value
    expect(info.mcpsCount).toBe(0)
    expect(info.skillsCount).toBe(0)
  })
})

describe('availableModes / availableModels caching', () => {
  it('records the most-recent availableModes advertisement', () => {
    pushSessionInfoUpdate('A', {
      availableModes: [
        { id: 'plan', name: 'Plan' },
        { id: 'edit', name: 'Edit' }
      ]
    })

    expect(useSessionInfo('A').info.value.availableModes).toEqual([
      { id: 'plan', name: 'Plan' },
      { id: 'edit', name: 'Edit' }
    ])
  })

  it('records the most-recent availableModels advertisement', () => {
    pushSessionInfoUpdate('A', {
      availableModels: [{ id: 'sonnet', name: 'Sonnet' }]
    })

    expect(useSessionInfo('A').info.value.availableModels).toEqual([{ id: 'sonnet', name: 'Sonnet' }])
  })

  it('replaces the prior advertisement wholesale', () => {
    pushSessionInfoUpdate('A', { availableModes: [{ id: 'plan', name: 'Plan' }] })
    pushSessionInfoUpdate('A', { availableModes: [{ id: 'edit', name: 'Edit' }] })

    const modes = useSessionInfo('A').info.value.availableModes
    expect(modes).toEqual([{ id: 'edit', name: 'Edit' }])
  })

  it('preserves prior advertisement when an update omits the field', () => {
    pushSessionInfoUpdate('A', { availableModels: [{ id: 'sonnet', name: 'Sonnet' }] })
    pushSessionInfoUpdate('A', { title: 'unrelated' })

    expect(useSessionInfo('A').info.value.availableModels).toEqual([{ id: 'sonnet', name: 'Sonnet' }])
  })

  it('records currentModelId as the active model', () => {
    pushSessionInfoUpdate('A', { currentModelId: 'opus' })

    expect(useSessionInfo('A').info.value.model).toBe('opus')
  })

  it('defaults availableModes / availableModels to empty arrays', () => {
    expect(useSessionInfo('A').info.value.availableModes).toEqual([])
    expect(useSessionInfo('A').info.value.availableModels).toEqual([])
  })
})

describe('setSessionRestored', () => {
  it('flips the restored flag and survives subsequent updates', () => {
    setSessionRestored('A', true)
    expect(useSessionInfo('A').info.value.restored).toBe(true)

    pushSessionInfoUpdate('A', { currentModeId: 'plan' })
    expect(useSessionInfo('A').info.value.restored).toBe(true)

    setSessionRestored('A', false)
    expect(useSessionInfo('A').info.value.restored).toBe(false)
  })

  it('defaults to false for instances that never saw setSessionRestored', () => {
    pushSessionInfoUpdate('A', { currentModeId: 'plan' })
    expect(useSessionInfo('A').info.value.restored).toBe(false)
  })
})

describe('truncateCwd', () => {
  it('returns short paths unchanged', () => {
    expect(truncateCwd('/tmp', 32)).toBe('/tmp')
  })

  it('collapses the home prefix to ~', () => {
    expect(truncateCwd('/home/cenk/dev/x', 32, '/home/cenk')).toBe('~/dev/x')
  })

  it('middle-ellipsises long paths keeping the head segment + last two', () => {
    const result = truncateCwd('/home/cenk/dev/utils/hyprpilot/src-tauri/src/adapters', 32, '/home/cenk')
    expect(result).toBe('~/.../src/adapters')
  })

  it('keeps the path as-is when middle truncation would not save bytes', () => {
    const path = '/a/b/c/d'
    expect(truncateCwd(path, 4)).toBe('/a/b/c/d')
  })

  it('does not collapse to ~ when home is the empty string', () => {
    expect(truncateCwd('/home/cenk/dev', 32, '')).toBe('/home/cenk/dev')
  })
})
