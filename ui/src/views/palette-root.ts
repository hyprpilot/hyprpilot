/**
 * Root palette specs. Every leaf under the root palette today is a
 * stub — opening one pushes a sub-palette with a single "not yet wired"
 * entry pointing at the Linear issue that will land the real content.
 *
 * This keeps the UX shape observable (nesting, Esc-to-pop, fuzzy filter)
 * without blocking on any follow-up wiring.
 */

import { type PaletteEntry, PaletteMode, type PaletteSpec, usePalette } from '@composables/palette'
import { log } from '@lib'

import { openProfilesLeaf } from './palette-profiles'
import { openSkillsLeaf } from './palette-skills'

/**
 * Closed set of root-palette leaf ids. Used by header-pill /
 * breadcrumb-click dispatch and the root palette's commit handler.
 * Adding a new leaf = new variant + new `ROOT_LEAVES` entry; the
 * `openRootLeaf` exhaustiveness check fails compile until both land.
 */
export enum PaletteLeafId {
  Sessions = 'sessions',
  Profiles = 'profiles',
  Models = 'models',
  Modes = 'modes',
  Commands = 'commands',
  Cwd = 'cwd',
  Permissions = 'permissions',
  Skills = 'skills',
  References = 'references',
  Mcps = 'mcps'
}

interface RootLeaf {
  id: PaletteLeafId
  name: string
  description: string
  followUp: string
}

const ROOT_LEAVES: Record<PaletteLeafId, RootLeaf> = {
  [PaletteLeafId.Sessions]: {
    id: PaletteLeafId.Sessions,
    name: 'sessions',
    description: 'resume a previous session',
    followUp: 'K-TBD'
  },
  [PaletteLeafId.Profiles]: {
    id: PaletteLeafId.Profiles,
    name: 'profiles',
    description: 'switch the active profile',
    followUp: 'K-TBD'
  },
  [PaletteLeafId.Models]: {
    id: PaletteLeafId.Models,
    name: 'models',
    description: 'pick a model override',
    followUp: 'K-TBD'
  },
  [PaletteLeafId.Modes]: {
    id: PaletteLeafId.Modes,
    name: 'modes',
    description: 'switch operational mode',
    followUp: 'K-TBD'
  },
  [PaletteLeafId.Commands]: {
    id: PaletteLeafId.Commands,
    name: 'commands',
    description: 'run a slash command',
    followUp: 'K-TBD'
  },
  [PaletteLeafId.Cwd]: {
    id: PaletteLeafId.Cwd,
    name: 'cwd',
    description: 'change the working directory',
    followUp: 'K-TBD'
  },
  [PaletteLeafId.Permissions]: {
    id: PaletteLeafId.Permissions,
    name: 'permissions',
    description: 'review permission rules',
    followUp: 'K-TBD'
  },
  [PaletteLeafId.Skills]: {
    id: PaletteLeafId.Skills,
    name: 'skills',
    description: 'attach a skill to the next prompt',
    followUp: 'K-269'
  },
  [PaletteLeafId.References]: {
    id: PaletteLeafId.References,
    name: 'references',
    description: 'insert a reference',
    followUp: 'K-TBD'
  },
  [PaletteLeafId.Mcps]: {
    id: PaletteLeafId.Mcps,
    name: 'mcps',
    description: 'toggle MCP servers',
    followUp: 'K-TBD'
  }
}

const ROOT_LEAF_ORDER: PaletteLeafId[] = [
  PaletteLeafId.Sessions,
  PaletteLeafId.Profiles,
  PaletteLeafId.Models,
  PaletteLeafId.Modes,
  PaletteLeafId.Commands,
  PaletteLeafId.Cwd,
  PaletteLeafId.Permissions,
  PaletteLeafId.Skills,
  PaletteLeafId.References,
  PaletteLeafId.Mcps
]

export function isPaletteLeafId(value: string): value is PaletteLeafId {
  return (Object.values(PaletteLeafId) as string[]).includes(value)
}

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
  const rootEntries: PaletteEntry[] = ROOT_LEAF_ORDER.map((id) => {
    const leaf = ROOT_LEAVES[id]

    return { id: leaf.id, name: leaf.name, description: leaf.description }
  })
  open({
    mode: PaletteMode.Select,
    title: 'palette',
    entries: rootEntries,
    onCommit(picks) {
      const pick = picks[0]
      if (!pick) {
        return
      }
      if (!isPaletteLeafId(pick.id)) {
        return
      }
      openRootLeaf(pick.id)
    }
  })
}

export function openSkillsPalette(): void {
  void openSkillsLeaf()
}

/**
 * Open one of the root leaves directly — used by header pill clicks
 * (cwd / mode / model / mcps / sessions). Each `case` swaps to a
 * real leaf as its issue lands; the `default` arm keeps the K-249
 * stub for the rest.
 */
export function openRootLeaf(leafId: PaletteLeafId): void {
  if (leafId === PaletteLeafId.Profiles) {
    openProfilesLeaf()

    return
  }
  if (leafId === PaletteLeafId.Skills) {
    openSkillsPalette()

    return
  }

  const { open } = usePalette()
  const leaf = ROOT_LEAVES[leafId]
  if (!leaf) {
    // Defensive: a new PaletteLeafId variant added without a ROOT_LEAVES
    // entry would land here. The exhaustiveness assertion below makes
    // that a compile-error first; this branch is the runtime safety net.
    log.warn('openRootLeaf: no ROOT_LEAVES entry for', { leafId })

    return
  }
  open(stubLeafSpec(leaf))
}
