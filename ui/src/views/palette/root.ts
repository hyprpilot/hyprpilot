/**
 * Root palette specs. Every leaf under the root palette dispatches to
 * its dedicated `open*Leaf()` exporter; stub leaves fall through to a
 * "not yet wired" placeholder pointing at the issue that will land
 * the real content.
 */

import { type PaletteEntry, PaletteMode, type PaletteSpec, usePalette } from '@composables'
import { log } from '@lib'

import { openCwdLeaf } from './cwd'
import { openInstanceLeaf } from './instance'
import { openInstancesLeaf } from './instances'
import { openMcpsLeaf, type OpenMcpsLeafOptions } from './mcps'
import { openModelsLeaf } from './models'
import { openModesLeaf } from './modes'
import { openProfilesLeaf } from './profiles'
import { openSessionsLeaf } from './sessions'
import { openSkillsLeaf } from './skills'

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
  Cwd = 'cwd',
  Instance = 'instance',
  Instances = 'instances',
  Permissions = 'permissions',
  Mcps = 'mcps',
  Skills = 'skills'
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
    followUp: 'K-264'
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
  [PaletteLeafId.Cwd]: {
    id: PaletteLeafId.Cwd,
    name: 'cwd',
    description: 'change the working directory',
    followUp: 'K-266'
  },
  [PaletteLeafId.Instance]: {
    id: PaletteLeafId.Instance,
    name: 'instance',
    description: 'rename / per-action on the focused instance',
    followUp: 'K-TBD'
  },
  [PaletteLeafId.Instances]: {
    id: PaletteLeafId.Instances,
    name: 'instances',
    description: 'switch / shut down a live instance',
    followUp: 'K-274'
  },
  [PaletteLeafId.Permissions]: {
    id: PaletteLeafId.Permissions,
    name: 'permissions',
    description: 'review permission rules',
    followUp: 'K-TBD'
  },
  [PaletteLeafId.Mcps]: {
    id: PaletteLeafId.Mcps,
    name: 'mcps',
    description: 'toggle MCP servers',
    followUp: 'K-TBD'
  },
  [PaletteLeafId.Skills]: {
    id: PaletteLeafId.Skills,
    name: 'skills',
    description: 'reload skills from disk',
    followUp: 'K-TBD'
  }
}

const ROOT_LEAF_ORDER: PaletteLeafId[] = [
  PaletteLeafId.Instance,
  PaletteLeafId.Instances,
  PaletteLeafId.Sessions,
  PaletteLeafId.Profiles,
  PaletteLeafId.Models,
  PaletteLeafId.Modes,
  PaletteLeafId.Cwd,
  PaletteLeafId.Permissions,
  PaletteLeafId.Mcps,
  PaletteLeafId.Skills
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

/**
 * Per-leaf context the dispatcher hands wired leaves at open time.
 * Stub leaves ignore the bag entirely — adding a field here without a
 * consumer is a no-op.
 */
export interface RootLeafContext {
  mcps?: OpenMcpsLeafOptions
}

/**
 * Open one of the root leaves directly — used by header pill clicks
 * (cwd / mode / model / mcps / sessions / instances). Wired leaves
 * dispatch to their dedicated `open*Leaf()` exporter; everything else
 * falls through to the K-249 stub spec.
 */
export function openRootLeaf(leafId: PaletteLeafId, ctx: RootLeafContext = {}): void {
  switch (leafId) {
    case PaletteLeafId.Sessions:
      void openSessionsLeaf()
      return
    case PaletteLeafId.Models:
      void openModelsLeaf()
      return
    case PaletteLeafId.Modes:
      void openModesLeaf()
      return
    case PaletteLeafId.Profiles:
      openProfilesLeaf()
      return
    case PaletteLeafId.Instance:
      void openInstanceLeaf()
      return
    case PaletteLeafId.Instances:
      void openInstancesLeaf()
      return
    case PaletteLeafId.Cwd:
      openCwdLeaf()
      return
    case PaletteLeafId.Mcps: {
      const { open } = usePalette()
      const leaf = ROOT_LEAVES[leafId]
      if (!ctx.mcps) {
        pushNoActiveInstanceStub(leaf, open)

        return
      }
      void openMcpsLeaf(ctx.mcps).catch((err) => {
        log.warn('openMcpsLeaf failed', { instanceId: ctx.mcps?.instanceId }, err)
      })
      return
    }
    case PaletteLeafId.Permissions: {
      const { open } = usePalette()
      const leaf = ROOT_LEAVES[leafId]
      open(stubLeafSpec(leaf))
      return
    }
    case PaletteLeafId.Skills:
      openSkillsLeaf()
      return
  }
}

function pushNoActiveInstanceStub(leaf: RootLeaf, open: ReturnType<typeof usePalette>['open']): void {
  open({
    mode: PaletteMode.Select,
    title: leaf.name,
    entries: [
      {
        id: `${leaf.id}-no-active-instance`,
        name: 'no active instance',
        description: 'spawn or focus an instance first'
      }
    ],
    onCommit: () => {}
  })
}
