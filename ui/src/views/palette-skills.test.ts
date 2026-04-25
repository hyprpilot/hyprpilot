import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { __resetPaletteStackForTests, type PaletteEntry, PaletteMode, usePalette } from '@composables/palette'
import { useAttachments } from '@composables/use-attachments'

const listSkills = vi.fn()
const getSkill = vi.fn()

vi.mock('@ipc', async () => {
  const actual = await vi.importActual<Record<string, unknown>>('@ipc')

  return {
    ...actual,
    listSkills: () => listSkills(),
    getSkill: (slug: string) => getSkill(slug)
  }
})

import { openSkillsLeaf } from './palette-skills'

const SKILL_A = {
  slug: 'alpha',
  title: 'Alpha',
  description: 'first skill'
}
const SKILL_B = {
  slug: 'beta',
  title: 'Beta',
  description: 'second skill'
}

const BODY_A = {
  slug: 'alpha',
  title: 'Alpha',
  description: 'first skill',
  body: '# alpha body',
  path: '/skills/alpha/SKILL.md',
  references: []
}
const BODY_B = {
  slug: 'beta',
  title: 'Beta',
  description: 'second skill',
  body: '# beta body',
  path: '/skills/beta/SKILL.md',
  references: []
}

beforeEach(() => {
  __resetPaletteStackForTests()
  useAttachments().clear()
  listSkills.mockReset()
  getSkill.mockReset()
})

afterEach(() => {
  __resetPaletteStackForTests()
  useAttachments().clear()
})

async function flushAsync(): Promise<void> {
  await Promise.resolve()
  await Promise.resolve()
  await Promise.resolve()
}

describe('openSkillsLeaf', () => {
  it('lists skills from skills_list and opens a multi-select palette', async () => {
    listSkills.mockResolvedValueOnce([SKILL_A, SKILL_B])

    await openSkillsLeaf()
    await flushAsync()

    const { stack } = usePalette()
    expect(stack.value).toHaveLength(1)
    const top = stack.value[0]!
    expect(top.mode).toBe(PaletteMode.MultiSelect)
    expect(top.title).toBe('skills')
    expect(top.entries.map((e) => e.id)).toEqual(['alpha', 'beta'])
  })

  it('pre-ticks entries that already live in the pending-attachments store', async () => {
    useAttachments().add({
      slug: 'alpha',
      path: '/skills/alpha/SKILL.md',
      body: '# alpha body',
      title: 'Alpha'
    })
    listSkills.mockResolvedValueOnce([SKILL_A, SKILL_B])

    await openSkillsLeaf()
    await flushAsync()

    const { stack } = usePalette()
    const top = stack.value[0]!
    expect(top.preseedActive?.map((e) => e.id)).toEqual(['alpha'])
  })

  it('commit fetches bodies for new picks and writes Attachment[] into the store', async () => {
    listSkills.mockResolvedValueOnce([SKILL_A, SKILL_B])
    getSkill.mockImplementation(async (slug: string) => (slug === 'alpha' ? BODY_A : BODY_B))

    await openSkillsLeaf()
    await flushAsync()

    const top = usePalette().stack.value[0]!
    const picks: PaletteEntry[] = [
      { id: 'alpha', name: 'Alpha' },
      { id: 'beta', name: 'Beta' }
    ]
    await top.onCommit(picks)
    await flushAsync()

    expect(getSkill).toHaveBeenCalledWith('alpha')
    expect(getSkill).toHaveBeenCalledWith('beta')
    const pending = useAttachments().pending.value
    expect(pending.map((a) => a.slug).sort()).toEqual(['alpha', 'beta'])
    expect(pending.find((a) => a.slug === 'alpha')?.body).toBe('# alpha body')
    expect(pending.find((a) => a.slug === 'beta')?.path).toBe('/skills/beta/SKILL.md')
  })

  it('commit drops un-ticked entries from the pending store', async () => {
    useAttachments().add({
      slug: 'alpha',
      path: '/skills/alpha/SKILL.md',
      body: '# alpha body',
      title: 'Alpha'
    })
    useAttachments().add({
      slug: 'beta',
      path: '/skills/beta/SKILL.md',
      body: '# beta body',
      title: 'Beta'
    })
    listSkills.mockResolvedValueOnce([SKILL_A, SKILL_B])

    await openSkillsLeaf()
    await flushAsync()
    const top = usePalette().stack.value[0]!

    // User commits with only `alpha` ticked → `beta` should drop.
    await top.onCommit([{ id: 'alpha', name: 'Alpha' }])
    await flushAsync()

    const pending = useAttachments().pending.value
    expect(pending.map((a) => a.slug)).toEqual(['alpha'])
    // `alpha` was already pending — body was NOT re-fetched on commit.
    expect(getSkill).not.toHaveBeenCalled()
  })

  it('esc-style close (palette pop without commit) leaves the pending store untouched', async () => {
    useAttachments().add({
      slug: 'alpha',
      path: '/skills/alpha/SKILL.md',
      body: '# alpha body',
      title: 'Alpha'
    })
    listSkills.mockResolvedValueOnce([SKILL_A, SKILL_B])

    await openSkillsLeaf()
    await flushAsync()

    // Simulate the CommandPalette.vue Esc key: pop the spec without
    // dispatching `onCommit`.
    usePalette().close()

    const pending = useAttachments().pending.value
    expect(pending.map((a) => a.slug)).toEqual(['alpha'])
    expect(usePalette().stack.value).toHaveLength(0)
  })

  it('skips picks whose body fetch fails (stale slug)', async () => {
    listSkills.mockResolvedValueOnce([SKILL_A, SKILL_B])
    getSkill.mockResolvedValueOnce(undefined).mockResolvedValueOnce(BODY_B)

    await openSkillsLeaf()
    await flushAsync()
    const top = usePalette().stack.value[0]!

    await top.onCommit([
      { id: 'alpha', name: 'Alpha' },
      { id: 'beta', name: 'Beta' }
    ])
    await flushAsync()

    const slugs = useAttachments().pending.value.map((a) => a.slug)
    expect(slugs).toEqual(['beta'])
  })
})
