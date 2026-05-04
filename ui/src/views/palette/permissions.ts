/**
 * Permissions palette leaf — multi-select review of the live
 * runtime trust store for the active instance. Lists every
 * `(tool, decision)` entry the captain has accumulated via "always
 * allow" / "always deny" buttons during this session; ticking a row
 * keeps the rule, unticking removes it. Decisions are colour-coded
 * via the `kind` slot (allow → ok, deny → err).
 *
 * Empty when the trust store carries nothing for the active
 * instance — captain typically hasn't pressed "always" on any
 * permission yet, or the active instance was just spawned (each
 * spawn / restart clears its slice). No rules to review = no
 * meaningful palette content; render the placeholder row so the
 * captain doesn't see a confusing empty list.
 */

import { ToastTone } from '@components'
import { type PaletteEntry, PaletteMode, useActiveInstance, usePalette, pushToast } from '@composables'
import { invoke, TauriCommand } from '@ipc'
import { log } from '@lib'

interface TrustEntry {
  tool: string
  decision: 'allow' | 'deny'
}

const EMPTY_ROW_ID = 'permissions-empty'

async function fetchTrustEntries(instanceId: string): Promise<TrustEntry[]> {
  try {
    const r = await invoke(TauriCommand.PermissionsTrustSnapshot, { instanceId })

    return r.entries
  } catch(err) {
    log.error('invoke failed', { command: TauriCommand.PermissionsTrustSnapshot }, err)
    pushToast(ToastTone.Err, `permissions snapshot failed: ${String(err)}`)

    return []
  }
}

async function forgetTrustEntry(instanceId: string, tool: string): Promise<void> {
  try {
    await invoke(TauriCommand.PermissionsTrustForget, { instanceId, tool })
  } catch(err) {
    log.error('invoke failed', { command: TauriCommand.PermissionsTrustForget, tool }, err)
    pushToast(ToastTone.Err, `permissions forget failed: ${String(err)}`)
  }
}

export async function openPermissionsLeaf(): Promise<void> {
  const { open } = usePalette()
  const { id: activeId } = useActiveInstance()
  const instanceId = activeId.value

  if (!instanceId) {
    open({
      mode: PaletteMode.Select,
      title: 'permissions',
      entries: [
        {
          id: EMPTY_ROW_ID,
          name: 'no active instance',
          description: 'no active instance.'
        }
      ],
      onCommit: () => {}
    })

    return
  }

  const entries = await fetchTrustEntries(instanceId)

  if (entries.length === 0) {
    open({
      mode: PaletteMode.Select,
      title: 'permissions',
      entries: [
        {
          id: EMPTY_ROW_ID,
          name: 'no rules.'
        }
      ],
      onCommit: () => {}
    })

    return
  }
  // Multi-select shape: every existing rule starts ticked. Unticking
  // and committing fires `forgetTrustEntry` for the dropped rows so
  // the next prompt for that tool re-asks. Ticked rows survive the
  // commit unchanged (idempotent — `remember` was already applied).
  const paletteEntries: PaletteEntry[] = entries.map((e) => ({
    id: `${e.decision}:${e.tool}`,
    name: e.tool,
    description: e.decision === 'allow' ? 'always allow' : 'always deny',
    kind: e.decision
  }))
  const allIds = paletteEntries.map((e) => ({ id: e.id, name: e.name }))

  open({
    mode: PaletteMode.MultiSelect,
    title: 'permissions',
    entries: paletteEntries,
    preseedActive: allIds,
    onCommit(picks) {
      const keptIds = new Set(picks.map((p) => p.id))
      const dropped = entries.filter((e) => !keptIds.has(`${e.decision}:${e.tool}`))

      if (dropped.length === 0) {
        return
      }
      Promise.all(dropped.map((e) => forgetTrustEntry(instanceId, e.tool)))
        .then(() => {
          pushToast(ToastTone.Ok, `dropped ${dropped.length} permission rule${dropped.length === 1 ? '' : 's'}`)
        })
        .catch(() => {})
    }
  })
}
