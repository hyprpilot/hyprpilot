/**
 * Instances palette leaf (K-274). Lists every live instance the
 * adapter knows about; `Enter` focuses one, `Ctrl+D` shuts it down.
 *
 * Row shape (captain-friendly):
 *   - `name`: captain-set name when set, else profile id, else agent id
 *   - `description`: `<adapter> · <model?>` plus phase / queue / terminal counts
 *   - `kind`: short instance-id slug (acts as a quiet handle in the row)
 *
 * Right pane: `InstancesPreview.vue` renders the headline + the last
 * two transcript turns so the captain can scan recent context without
 * focusing the instance first.
 */

import InstancesPreview from './InstancesPreview.vue'
import { ToastTone } from '@components'
import { type PaletteEntry, PaletteMode, type PaletteSpec, usePalette, useActiveInstance, type InstanceId } from '@composables'
import { useHomeDir, usePhase, useQueue, useSessionInfo, useTerminals, pushToast } from '@composables'
import { TauriCommand } from '@ipc'
import { type InstanceListEntry } from '@ipc'
import { invoke } from '@ipc/bridge'
import { log } from '@lib'

interface InstanceRow extends PaletteEntry {
  raw: InstanceListEntry
}

function rowFor(entry: InstanceListEntry, displayPath: (path: string | undefined) => string): InstanceRow {
  const { id: activeId } = useActiveInstance()
  const { info } = useSessionInfo(entry.instanceId)
  const { items } = useQueue(entry.instanceId)
  const { all: terminals } = useTerminals(entry.instanceId)
  const { phase } = usePhase(entry.instanceId)

  // Headline name: captain-renamed → profile id → adapter id.
  const headline = entry.name ?? entry.profileId ?? entry.agentId

  // Description groups: adapter / model first (the wireframe ask), then
  // phase + cwd + counts so fuzzy filter still hits every signal.
  const meta: string[] = [entry.agentId]
  const model = info.value.model

  if (model) {
    meta.push(model)
  }
  meta.push(phase.value)
  const cwd = info.value.cwd

  if (cwd) {
    // Display-friendly: home → ~ substitution. Chrome's CSS
    // `text-overflow: ellipsis` handles overflow at row width.
    meta.push(displayPath(cwd))
  }

  if (info.value.mode) {
    meta.push(info.value.mode)
  }

  if (items.value.length > 0) {
    meta.push(`q${items.value.length}`)
  }

  if (terminals.value.length > 0) {
    meta.push(`t${terminals.value.length}`)
  }

  if (entry.instanceId === activeId.value) {
    meta.unshift('active')
  }

  return {
    id: entry.instanceId,
    name: headline,
    description: meta.join(' · '),
    kind: entry.instanceId.slice(0, 8),
    raw: entry
  }
}

async function fetchInstances(): Promise<InstanceListEntry[]> {
  try {
    const r = await invoke(TauriCommand.InstancesList)

    return r.instances
  } catch(err) {
    log.error('invoke failed', { command: TauriCommand.InstancesList }, err)
    pushToast(ToastTone.Err, `instances list failed: ${String(err)}`)

    return []
  }
}

async function focusInstance(id: InstanceId): Promise<void> {
  try {
    await invoke(TauriCommand.InstancesFocus, { id })
  } catch(err) {
    log.error('invoke failed', { command: TauriCommand.InstancesFocus, id }, err)
    pushToast(ToastTone.Err, `instances focus failed: ${String(err)}`)
  }
}

export async function shutdownInstance(id: InstanceId): Promise<void> {
  try {
    await invoke(TauriCommand.InstancesShutdown, { id })
  } catch(err) {
    log.error('invoke failed', { command: TauriCommand.InstancesShutdown, id }, err)
    pushToast(ToastTone.Err, `instances shutdown failed: ${String(err)}`)
  }
}

export async function openInstancesLeaf(): Promise<void> {
  const palette = usePalette()
  const { displayPath } = useHomeDir()

  const instances = await fetchInstances()

  if (instances.length === 0) {
    palette.open({
      mode: PaletteMode.Select,
      title: 'instances',
      entries: [
        {
          id: 'instances-empty',
          name: 'no live instances.'
        }
      ],
      onCommit: () => {}
    })

    return
  }

  const entries: PaletteEntry[] = instances.map((i) => rowFor(i, displayPath))
  const spec = {
    mode: PaletteMode.Select,
    title: 'instances',
    entries,
    preview: {
      component: InstancesPreview,
      props: { items: instances }
    },
    onCommit(picks: PaletteEntry[]) {
      const pick = picks[0]

      if (!pick || pick.id === 'instances-empty') {
        return
      }
      void focusInstance(pick.id)
    },
    async onDelete(entry: PaletteEntry, update: (entries: PaletteEntry[]) => void) {
      if (entry.id === 'instances-empty') {
        return
      }
      await shutdownInstance(entry.id)
      // Re-fetch + push through the reactive `update` callback so
      // the captain sees the updated registry without re-opening
      // the palette. Mutating `spec.entries = ...` on the captured
      // literal bypasses Vue's proxy — usePaletteFilter never re-
      // fires and the row list goes stale.
      const next = await fetchInstances()

      if (next.length === 0) {
        update([
          {
            id: 'instances-empty',
            name: 'no live instances.'
          }
        ])

        return
      }
      update(next.map((i) => rowFor(i, displayPath)))
      // Preview's a separate component instance bound via spec.preview;
      // its data lands through `props` which already reads from
      // `instances` via the InstancesPreview component. Re-binding
      // here means the preview-pane updates alongside the row list.
      spec.preview.props.items = next
    }
  } satisfies PaletteSpec

  palette.open(spec)
}
