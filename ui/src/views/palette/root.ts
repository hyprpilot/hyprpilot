/**
 * Root palette specs. Every leaf under the root palette dispatches to
 * its dedicated `open*Leaf()` exporter; stub leaves fall through to a
 * "not yet wired" placeholder pointing at the issue that will land
 * the real content.
 */

import { openCwdLeaf } from './cwd'
import { openDaemonLeaf } from './daemon'
import { openInstanceLeaf } from './instance'
import { openInstancesLeaf } from './instances'
import { openMcpsLeaf, type OpenMcpsLeafOptions } from './mcps'
import { openModelsLeaf } from './models'
import { openModesLeaf } from './modes'
import { openProfilesLeaf } from './profiles'
import { openSessionsLeaf } from './sessions'
import { openSkillsLeaf } from './skills'
import { type PaletteEntry, PaletteMode, useActiveInstance, usePalette } from '@composables'
import { log } from '@lib'

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
  Mcps = 'mcps',
  Skills = 'skills',
  Daemon = 'daemon'
}

interface RootLeaf {
  id: PaletteLeafId
  name: string
  followUp?: string
}

const ROOT_LEAVES: Record<PaletteLeafId, RootLeaf> = {
  [PaletteLeafId.Sessions]: {
    id: PaletteLeafId.Sessions,
    name: 'sessions',
    followUp: 'K-264'
  },
  [PaletteLeafId.Profiles]: {
    id: PaletteLeafId.Profiles,
    name: 'profiles',
    followUp: 'K-TBD'
  },
  [PaletteLeafId.Models]: {
    id: PaletteLeafId.Models,
    name: 'models',
    followUp: 'K-TBD'
  },
  [PaletteLeafId.Modes]: {
    id: PaletteLeafId.Modes,
    name: 'modes',
    followUp: 'K-TBD'
  },
  [PaletteLeafId.Cwd]: {
    id: PaletteLeafId.Cwd,
    name: 'cwd',
    followUp: 'K-266'
  },
  [PaletteLeafId.Instance]: {
    id: PaletteLeafId.Instance,
    name: 'instance',
    followUp: 'K-TBD'
  },
  [PaletteLeafId.Instances]: {
    id: PaletteLeafId.Instances,
    name: 'instances',
    followUp: 'K-274'
  },
  [PaletteLeafId.Mcps]: {
    id: PaletteLeafId.Mcps,
    name: 'mcps',
    followUp: 'K-TBD'
  },
  [PaletteLeafId.Skills]: {
    id: PaletteLeafId.Skills,
    name: 'skills',
    followUp: 'K-TBD'
  },
  [PaletteLeafId.Daemon]: {
    id: PaletteLeafId.Daemon,
    name: 'daemon'
  }
}

const ROOT_LEAF_ORDER: PaletteLeafId[] = [
  PaletteLeafId.Instance,
  PaletteLeafId.Instances,
  PaletteLeafId.Profiles,
  PaletteLeafId.Sessions,
  PaletteLeafId.Models,
  PaletteLeafId.Modes,
  PaletteLeafId.Cwd,
  PaletteLeafId.Mcps,
  PaletteLeafId.Skills,
  // Daemon stays last — it's the captain's wire-side surface for
  // ops actions (reload / shutdown / status snapshot) and isn't
  // part of the per-instance navigation flow above it.
  PaletteLeafId.Daemon
]

export function isPaletteLeafId(value: string): value is PaletteLeafId {
  return (Object.values(PaletteLeafId) as string[]).includes(value)
}

export function openRootPalette(): void {
  const { open } = usePalette()
  const rootEntries: PaletteEntry[] = ROOT_LEAF_ORDER.map((id) => {
    const leaf = ROOT_LEAVES[id]

    return {
      id: leaf.id,
      name: leaf.name
    }
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
      // Resolve the addressed instance from the explicit ctx (header
      // breadcrumb path) or fall through to the live active instance
      // (root-palette path — `Ctrl+K → mcps` carries no ctx). Without
      // this the root leaf always saw `ctx.mcps === undefined` and
      // dead-ended at the empty stub even when an instance was live.
      const resolved = ctx.mcps ?? (useActiveInstance().id.value ? { instanceId: useActiveInstance().id.value as string } : undefined)

      if (!resolved) {
        pushNoActiveInstanceStub(leaf, open)

        return
      }
      void openMcpsLeaf(resolved).catch((err) => {
        log.warn('openMcpsLeaf failed', { instanceId: resolved.instanceId }, err)
      })

      return
    }

    case PaletteLeafId.Skills:
      openSkillsLeaf()

      return

    case PaletteLeafId.Daemon:
      openDaemonLeaf()

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
        name: 'no active instance.'
      }
    ],
    onCommit: () => {}
  })
}
