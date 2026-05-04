import { beforeEach, describe, expect, it, vi } from 'vitest'
import { ref } from 'vue'

import { buildProfilesLeafEntries, buildProfilesPaletteSpec, openProfilesLeaf } from './profiles'
import { __resetPaletteStackForTests, usePalette } from '@composables'

const selectMock = vi.fn()
const profilesRef = ref<{ id: string; agent: string; model?: string; isDefault: boolean }[]>([])
const selectedRef = ref<string | undefined>(undefined)
const loadingRef = ref(false)
const activeInstanceRef = ref<string | undefined>(undefined)
const pushToastMock = vi.fn()

vi.mock('@composables', async(importOriginal) => ({
  ...(await importOriginal<typeof import('@composables')>()),
  useProfiles: () => ({
    profiles: profilesRef,
    selected: selectedRef,
    loading: loadingRef,
    select: selectMock
  }),
  useActiveInstance: () => ({
    id: activeInstanceRef
  }),
  pushToast: (...args: unknown[]) => pushToastMock(...args)
}))

beforeEach(() => {
  __resetPaletteStackForTests()
  selectMock.mockReset()
  pushToastMock.mockReset()
  profilesRef.value = []
  selectedRef.value = undefined
  loadingRef.value = false
  activeInstanceRef.value = undefined
})

describe('buildProfilesLeafEntries', () => {
  it('marks the selected profile with the active kind tag', () => {
    const { entries, activeId } = buildProfilesLeafEntries({
      list: [
        {
          id: 'ask',
          agent: 'claude-code',
          model: 'sonnet',
          isDefault: true
        },
        {
          id: 'strict',
          agent: 'claude-code',
          model: 'opus',
          isDefault: false
        }
      ],
      selected: 'strict'
    })

    expect(activeId).toBe('strict')
    expect(entries).toHaveLength(2)
    expect(entries[0]?.id).toBe('ask')
    expect(entries[0]?.kind).toBe('default')
    expect(entries[1]?.id).toBe('strict')
    expect(entries[1]?.kind).toBe('active')
  })

  it('falls back to default tag when no profile is selected', () => {
    const { entries, activeId } = buildProfilesLeafEntries({
      list: [
        {
          id: 'ask',
          agent: 'claude-code',
          isDefault: true
        },
        {
          id: 'strict',
          agent: 'claude-code',
          isDefault: false
        }
      ]
    })

    expect(activeId).toBeUndefined()
    expect(entries[0]?.kind).toBe('default')
    expect(entries[1]?.kind).toBeUndefined()
  })

  it('joins agent + model into the description, with em-dash for missing model', () => {
    const { entries } = buildProfilesLeafEntries({
      list: [
        {
          id: 'ask',
          agent: 'claude-code',
          model: 'sonnet',
          isDefault: true
        },
        {
          id: 'strict',
          agent: 'codex',
          isDefault: false
        }
      ]
    })

    expect(entries[0]?.description).toBe('claude-code · sonnet')
    expect(entries[1]?.description).toBe('codex · —')
  })
})

describe('buildProfilesPaletteSpec', () => {
  it('onCommit dispatches the picked id to onSelect', () => {
    const onSelect = vi.fn()
    const spec = buildProfilesPaletteSpec({
      list: [
        {
          id: 'ask',
          agent: 'claude-code',
          isDefault: true
        },
        {
          id: 'strict',
          agent: 'claude-code',
          isDefault: false
        }
      ],
      selected: 'ask',
      onSelect
    })

    spec.onCommit([{ id: 'strict', name: 'strict' }])
    expect(onSelect).toHaveBeenCalledWith('strict')
  })

  it('onCommit no-ops when the picked id matches the active id', () => {
    const onSelect = vi.fn()
    const spec = buildProfilesPaletteSpec({
      list: [
        {
          id: 'ask',
          agent: 'claude-code',
          isDefault: true
        }
      ],
      selected: 'ask',
      onSelect
    })

    spec.onCommit([{ id: 'ask', name: 'ask' }])
    expect(onSelect).not.toHaveBeenCalled()
  })

  it('onDelete surfaces a not-yet-wired toast (K-280) and never calls onSelect', () => {
    const onSelect = vi.fn()
    const spec = buildProfilesPaletteSpec({
      list: [
        {
          id: 'ask',
          agent: 'claude-code',
          isDefault: true
        }
      ],
      selected: 'ask',
      onSelect
    })

    spec.onDelete?.({ id: 'ask', name: 'ask' }, () => {})
    expect(onSelect).not.toHaveBeenCalled()
    expect(pushToastMock).toHaveBeenCalledTimes(1)
    expect(pushToastMock.mock.calls[0]?.[1]).toMatch(/K-280/)
  })
})

describe('openProfilesLeaf', () => {
  it('pushes the profiles spec onto the palette stack with profiles loaded', () => {
    profilesRef.value = [
      {
        id: 'ask',
        agent: 'claude-code',
        model: 'sonnet',
        isDefault: true
      },
      {
        id: 'strict',
        agent: 'claude-code',
        isDefault: false
      }
    ]
    selectedRef.value = 'ask'

    openProfilesLeaf()

    const { stack } = usePalette()

    expect(stack.value).toHaveLength(1)
    const spec = stack.value[0]

    expect(spec?.title).toBe('profiles')
    expect(spec?.entries.map((e) => e.id)).toEqual(['ask', 'strict'])
    expect(spec?.entries[0]?.kind).toBe('active')
  })

  it('toasts "still loading" and bails when the registry is mid-fetch', () => {
    profilesRef.value = []
    loadingRef.value = true

    openProfilesLeaf()

    const { stack } = usePalette()

    expect(stack.value).toHaveLength(0)
    expect(pushToastMock).toHaveBeenCalledTimes(1)
    expect(pushToastMock.mock.calls[0]?.[1]).toMatch(/still loading/i)
  })

  it('toasts "none configured" and bails when the registry is fetched but empty', () => {
    profilesRef.value = []
    loadingRef.value = false

    openProfilesLeaf()

    const { stack } = usePalette()

    expect(stack.value).toHaveLength(0)
    expect(pushToastMock).toHaveBeenCalledTimes(1)
    expect(pushToastMock.mock.calls[0]?.[1]).toMatch(/none configured/i)
  })

  it('committing a different row routes through useProfiles().select and toasts ok', () => {
    profilesRef.value = [
      {
        id: 'ask',
        agent: 'claude-code',
        isDefault: true
      },
      {
        id: 'strict',
        agent: 'claude-code',
        isDefault: false
      }
    ]
    selectedRef.value = 'ask'

    openProfilesLeaf()
    const { stack } = usePalette()
    const spec = stack.value[0]

    spec?.onCommit([{ id: 'strict', name: 'strict' }])

    expect(selectMock).toHaveBeenCalledWith('strict')
    expect(pushToastMock).toHaveBeenCalledWith('ok', 'profile: strict')
  })
})
