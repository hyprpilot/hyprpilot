/**
 * Root palette specs. Every leaf under the root palette today is a
 * stub — opening one pushes a sub-palette with a single "not yet wired"
 * entry pointing at the Linear issue that will land the real content.
 *
 * This keeps the UX shape observable (nesting, Esc-to-pop, fuzzy filter)
 * without blocking on any follow-up wiring.
 */

import { type PaletteEntry, PaletteMode, type PaletteSpec, usePalette } from '@composables/palette'

interface RootLeaf {
  id: string
  name: string
  description: string
  followUp: string
}

const ROOT_LEAVES: RootLeaf[] = [
  { id: 'sessions', name: 'sessions', description: 'resume a previous session', followUp: 'K-TBD' },
  { id: 'profiles', name: 'profiles', description: 'switch the active profile', followUp: 'K-TBD' },
  { id: 'models', name: 'models', description: 'pick a model override', followUp: 'K-TBD' },
  { id: 'modes', name: 'modes', description: 'switch operational mode', followUp: 'K-TBD' },
  { id: 'commands', name: 'commands', description: 'run a slash command', followUp: 'K-TBD' },
  { id: 'cwd', name: 'cwd', description: 'change the working directory', followUp: 'K-TBD' },
  { id: 'permissions', name: 'permissions', description: 'review permission rules', followUp: 'K-TBD' },
  { id: 'skills', name: 'skills', description: 'browse skills catalog', followUp: 'K-TBD' },
  { id: 'references', name: 'references', description: 'insert a reference', followUp: 'K-TBD' },
  { id: 'mcps', name: 'mcps', description: 'toggle MCP servers', followUp: 'K-TBD' }
]

function stubLeafSpec(leaf: RootLeaf): PaletteSpec {
  return {
    mode: PaletteMode.Select,
    title: leaf.name,
    entries: [
      {
        id: `${leaf.id}-placeholder`,
        name: `not yet wired — see ${leaf.followUp}`,
        description: leaf.description
      }
    ],
    onCommit: () => {}
  }
}

export function openRootPalette(): void {
  const { open } = usePalette()
  const rootEntries: PaletteEntry[] = ROOT_LEAVES.map((leaf) => ({
    id: leaf.id,
    name: leaf.name,
    description: leaf.description
  }))
  open({
    mode: PaletteMode.Select,
    title: 'palette',
    entries: rootEntries,
    onCommit(picks) {
      const pick = picks[0]
      if (!pick) {
        return
      }
      const leaf = ROOT_LEAVES.find((l) => l.id === pick.id)
      if (!leaf) {
        return
      }
      open(stubLeafSpec(leaf))
    }
  })
}

export function openSkillsPalette(): void {
  const { open } = usePalette()
  open({
    mode: PaletteMode.MultiSelect,
    title: 'skills',
    entries: [
      {
        id: 'skills-placeholder',
        name: 'not yet wired — see K-268',
        description: 'browse skills catalog'
      }
    ],
    onCommit: () => {}
  })
}
