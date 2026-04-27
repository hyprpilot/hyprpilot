/**
 * Instances palette leaf (K-274). Lists every live instance the
 * adapter knows about; `Enter` focuses one, `Ctrl+D` shuts it down.
 *
 * Single column today — `CommandPalette.vue` doesn't expose a
 * preview-pane slot, so the per-row `description` carries
 * `<phase> · <cwd-short> · q<queue> · t<terminals>` so the palette's
 * fuzzy filter still matches across every signal.
 */

import { type PaletteEntry, PaletteMode, usePalette } from '@composables/palette'
import { useActiveInstance, type InstanceId } from '@composables/use-active-instance'
import { useHomeDir } from '@composables/use-home-dir'
import { usePhase } from '@composables/use-phase'
import { useQueue } from '@composables/use-queue'
import { truncateCwd, useSessionInfo } from '@composables/use-session-info'
import { useTerminals } from '@composables/use-terminals'
import { pushToast } from '@composables/use-toasts'

import { ToastTone } from '@components/types'

import { invoke } from '@ipc/bridge'
import { TauriCommand } from '@ipc/commands'
import { type InstanceListEntry } from '@ipc/types'
import { log } from '@lib'

interface InstanceRow extends PaletteEntry {
  raw: InstanceListEntry
}

function rowFor(entry: InstanceListEntry, homeDir: string | undefined): InstanceRow {
  const { id: activeId } = useActiveInstance()
  const { info } = useSessionInfo(entry.instanceId)
  const { items } = useQueue(entry.instanceId)
  const { all: terminals } = useTerminals(entry.instanceId)
  const { phase } = usePhase(entry.instanceId)

  const segments: string[] = [phase.value]
  const cwd = info.value.cwd
  if (cwd) {
    segments.push(truncateCwd(cwd, 40, homeDir))
  }
  if (info.value.mode) {
    segments.push(info.value.mode)
  }
  if (items.value.length > 0) {
    segments.push(`q${items.value.length}`)
  }
  if (terminals.value.length > 0) {
    segments.push(`t${terminals.value.length}`)
  }
  if (entry.instanceId === activeId.value) {
    segments.unshift('active')
  }

  const profileLabel = entry.profileId ?? 'no-profile'
  const name = `${entry.agentId} · ${profileLabel}`

  return {
    id: entry.instanceId,
    name,
    description: segments.join(' · '),
    kind: entry.instanceId.slice(0, 8),
    raw: entry
  }
}

async function fetchInstances(): Promise<InstanceListEntry[]> {
  try {
    const r = await invoke(TauriCommand.InstancesList)

    return r.instances
  } catch (err) {
    log.error('invoke failed', { command: TauriCommand.InstancesList }, err)
    pushToast(ToastTone.Err, `instances list failed: ${String(err)}`)

    return []
  }
}

async function focusInstance(id: InstanceId): Promise<void> {
  try {
    await invoke(TauriCommand.InstancesFocus, { id })
  } catch (err) {
    log.error('invoke failed', { command: TauriCommand.InstancesFocus, id }, err)
    pushToast(ToastTone.Err, `instances focus failed: ${String(err)}`)
  }
}

async function shutdownInstance(id: InstanceId): Promise<void> {
  try {
    await invoke(TauriCommand.InstancesShutdown, { id })
  } catch (err) {
    log.error('invoke failed', { command: TauriCommand.InstancesShutdown, id }, err)
    pushToast(ToastTone.Err, `instances shutdown failed: ${String(err)}`)
  }
}

export async function openInstancesLeaf(): Promise<void> {
  const { open } = usePalette()
  const { homeDir } = useHomeDir()

  const instances = await fetchInstances()

  if (instances.length === 0) {
    open({
      mode: PaletteMode.Select,
      title: 'instances',
      entries: [
        {
          id: 'instances-empty',
          name: 'no live instances',
          description: 'submit a prompt to spawn one'
        }
      ],
      onCommit: () => {}
    })

    return
  }

  const entries: InstanceRow[] = instances.map((i) => rowFor(i, homeDir.value))

  open({
    mode: PaletteMode.Select,
    title: 'instances',
    entries,
    onCommit(picks) {
      const pick = picks[0]
      if (!pick || pick.id === 'instances-empty') {
        return
      }
      void focusInstance(pick.id)
    },
    onDelete(entry) {
      if (entry.id === 'instances-empty') {
        return
      }
      void shutdownInstance(entry.id)
    }
  })
}
