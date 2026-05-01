import { describe, expect, it } from 'vitest'
import { ref } from 'vue'

import { type PaletteEntry, PaletteMode, type PaletteSpec } from './palette'
import { usePaletteFilter } from './use-palette-filter'

const entries: PaletteEntry[] = [
  { id: 'git-status', name: 'git status' },
  { id: 'git-stash', name: 'git stash' },
  { id: 'docker-up', name: 'docker compose up' },
  { id: 'help', name: 'help' }
]

function specOf(mode: PaletteMode = PaletteMode.Select): PaletteSpec {
  return {
    mode,
    entries,
    onCommit: () => {}
  }
}

describe('usePaletteFilter', () => {
  it('returns the spec entries verbatim when the query is empty', () => {
    const spec = ref<PaletteSpec | undefined>(specOf())
    const query = ref('')
    const ticked = ref<Set<string>>(new Set())
    const { visible } = usePaletteFilter(spec, query, ticked)
    expect(visible.value.map((e) => e.id)).toEqual(entries.map((e) => e.id))
  })

  it('subsequence-prefilters out non-matches', () => {
    const spec = ref<PaletteSpec | undefined>(specOf())
    const query = ref('gst')
    const ticked = ref<Set<string>>(new Set())
    const { visible } = usePaletteFilter(spec, query, ticked)
    const ids = visible.value.map((e) => e.id)
    expect(ids).toContain('git-status')
    expect(ids).toContain('git-stash')
    expect(ids).not.toContain('help')
  })

  it('pins ticked rows to the top in multi-select mode', () => {
    const spec = ref<PaletteSpec | undefined>(specOf(PaletteMode.MultiSelect))
    const query = ref('')
    const ticked = ref<Set<string>>(new Set(['help']))
    const { visible } = usePaletteFilter(spec, query, ticked)
    expect(visible.value[0]?.id).toBe('help')
  })

  it('returns empty when no spec is on top', () => {
    const spec = ref<PaletteSpec | undefined>(undefined)
    const query = ref('')
    const ticked = ref<Set<string>>(new Set())
    const { visible } = usePaletteFilter(spec, query, ticked)
    expect(visible.value).toEqual([])
  })
})
