import { beforeEach, describe, expect, it, vi } from 'vitest'

import {
  __resetAllSessionInfoForTests,
  pushCurrentModeUpdate,
  pushInstanceModeState,
  pushInstanceModelState,
  pushSessionInfoUpdate,
  setInstanceCwd,
  setInstanceGitStatus,
  setSessionRestored,
  truncateCwd,
  useActiveInstance,
  useSessionInfo
} from '@composables'

const profilesRef = { value: [] as { id: string; agent: string; model?: string; isDefault: boolean }[] }
const selectedRef = { value: undefined as string | undefined }

vi.mock('../ui-state/use-profiles', () => ({
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

describe('pushSessionInfoUpdate (ACP SessionInfoUpdate)', () => {
  it('records the title when session_info_update arrives', () => {
    pushSessionInfoUpdate('A', { title: 'fix the build' })

    expect(useSessionInfo('A').info.value.title).toBe('fix the build')
  })

  it('clears the title when an explicit empty string arrives', () => {
    pushSessionInfoUpdate('A', { title: 'first' })
    pushSessionInfoUpdate('A', { title: '' })

    expect(useSessionInfo('A').info.value.title).toBe('')
  })

  it('records updatedAt alongside title', () => {
    pushSessionInfoUpdate('A', { title: 't', updatedAt: '2026-04-30T10:00:00Z' })

    const info = useSessionInfo('A').info.value

    expect(info.title).toBe('t')
    expect(info.updatedAt).toBe('2026-04-30T10:00:00Z')
  })

  it('preserves prior title when an update omits the field', () => {
    pushSessionInfoUpdate('A', { title: 'first' })
    pushSessionInfoUpdate('A', { updatedAt: '2026-04-30T10:00:00Z' })

    expect(useSessionInfo('A').info.value.title).toBe('first')
  })
})

describe('pushCurrentModeUpdate (ACP CurrentModeUpdate)', () => {
  it('records the mode for the addressed instance', () => {
    pushCurrentModeUpdate('A', { currentModeId: 'plan' })

    useActiveInstance().id.value = 'A'
    expect(useSessionInfo().info.value.mode).toBe('plan')
  })

  it('isolates state between instances', () => {
    pushCurrentModeUpdate('A', { currentModeId: 'plan' })
    pushCurrentModeUpdate('B', { currentModeId: 'edit' })

    expect(useSessionInfo('A').info.value.mode).toBe('plan')
    expect(useSessionInfo('B').info.value.mode).toBe('edit')
  })
})

describe('pushInstanceModeState (NewSessionResponse.modes)', () => {
  it('records the most-recent availableModes advertisement', () => {
    pushInstanceModeState('A', {
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

  it('replaces the prior advertisement wholesale', () => {
    pushInstanceModeState('A', { availableModes: [{ id: 'plan', name: 'Plan' }] })
    pushInstanceModeState('A', { availableModes: [{ id: 'edit', name: 'Edit' }] })

    expect(useSessionInfo('A').info.value.availableModes).toEqual([{ id: 'edit', name: 'Edit' }])
  })

  it('also seeds the currentModeId when present', () => {
    pushInstanceModeState('A', {
      currentModeId: 'plan',
      availableModes: [{ id: 'plan', name: 'Plan' }]
    })

    expect(useSessionInfo('A').info.value.mode).toBe('plan')
  })
})

describe('pushInstanceModelState (NewSessionResponse.models)', () => {
  it('records the most-recent availableModels advertisement', () => {
    pushInstanceModelState('A', { availableModels: [{ id: 'sonnet', name: 'Sonnet' }] })

    expect(useSessionInfo('A').info.value.availableModels).toEqual([{ id: 'sonnet', name: 'Sonnet' }])
  })

  it('records currentModelId as the active model', () => {
    pushInstanceModelState('A', { currentModelId: 'opus' })

    expect(useSessionInfo('A').info.value.model).toBe('opus')
  })
})

describe('setInstanceCwd / setInstanceGitStatus (daemon-side metadata)', () => {
  it('records and clears cwd', () => {
    setInstanceCwd('A', '/home/me/dev/proj')
    expect(useSessionInfo('A').info.value.cwd).toBe('/home/me/dev/proj')
  })

  it('records and clears gitStatus', () => {
    setInstanceGitStatus('A', {
      branch: 'main',
      ahead: 2,
      behind: 0
    })
    expect(useSessionInfo('A').info.value.gitStatus).toEqual({
      branch: 'main',
      ahead: 2,
      behind: 0
    })

    setInstanceGitStatus('A', undefined)
    expect(useSessionInfo('A').info.value.gitStatus).toBeUndefined()
  })
})

describe('useSessionInfo profile derivation', () => {
  it('derives model from the active profile when the instance has no override', () => {
    profilesRef.value = [
      {
        id: 'ask',
        agent: 'claude-code',
        model: 'claude-sonnet-4-5',
        isDefault: true
      }
    ]
    selectedRef.value = 'ask'

    expect(useSessionInfo('A').info.value.model).toBe('claude-sonnet-4-5')
  })

  it('prefers the instance model over the profile model when both exist', () => {
    profilesRef.value = [
      {
        id: 'ask',
        agent: 'claude-code',
        model: 'claude-sonnet-4-5',
        isDefault: true
      }
    ]
    selectedRef.value = 'ask'
    pushInstanceModelState('A', { currentModelId: 'claude-opus-4-5' })

    expect(useSessionInfo('A').info.value.model).toBe('claude-opus-4-5')
  })

  it('always reports zero mcps and skills counts (live counts land in K-258 / K-268)', () => {
    profilesRef.value = [
      {
        id: 'ask',
        agent: 'claude-code',
        isDefault: true
      }
    ]
    selectedRef.value = 'ask'

    const info = useSessionInfo('A').info.value

    expect(info.mcpsCount).toBe(0)
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

    pushCurrentModeUpdate('A', { currentModeId: 'plan' })
    expect(useSessionInfo('A').info.value.restored).toBe(true)

    setSessionRestored('A', false)
    expect(useSessionInfo('A').info.value.restored).toBe(false)
  })

  it('defaults to false for instances that never saw setSessionRestored', () => {
    pushCurrentModeUpdate('A', { currentModeId: 'plan' })
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
