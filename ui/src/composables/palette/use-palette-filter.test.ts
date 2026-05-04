import { flushPromises } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { nextTick, ref } from 'vue'

import { type PaletteEntry, PaletteMode, type PaletteSpec } from './palette'
import { usePaletteFilter } from './use-palette-filter'

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }))

vi.mock('@ipc/bridge', async() => ({
  ...(await vi.importActual<object>('@ipc/bridge')),
  invoke: (command: string, args?: Record<string, unknown>) => invoke(command, args),
  listen: vi.fn()
}))

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

/// Wait long enough for the 60ms debounce timer + the awaited
/// `completion/rank` call to settle the watcher's `ranked` ref.
async function settleRanker(): Promise<void> {
  await new Promise((resolve) => setTimeout(resolve, 80))
  await flushPromises()
  await nextTick()
}

beforeEach(() => {
  invoke.mockReset()
})

describe('usePaletteFilter', () => {
  it('returns the spec entries verbatim when the query is empty', () => {
    const spec = ref<PaletteSpec | undefined>(specOf())
    const query = ref('')
    const ticked = ref<Set<string>>(new Set())
    const { visible } = usePaletteFilter(spec, query, ticked)

    expect(visible.value.map((e) => e.id)).toEqual(entries.map((e) => e.id))
  })

  it('routes non-empty query through completion/rank and renders the daemon-ranked order', async() => {
    invoke.mockImplementation(async(_command, args) => {
      const { candidates } = args as { query: string; candidates: { id: string; label: string }[] }
      const matched = candidates.filter((c) => c.id === 'git-status' || c.id === 'git-stash')

      return {
        items: matched.map((c) => ({
          label: c.label,
          replacement: { range: { start: 0, end: 3 }, text: c.id }
        }))
      }
    })

    const spec = ref<PaletteSpec | undefined>(specOf())
    const query = ref('gst')
    const ticked = ref<Set<string>>(new Set())
    const { visible } = usePaletteFilter(spec, query, ticked)

    await settleRanker()

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
