/**
 * Instances palette leaf. Lists every live instance the adapter
 * knows about; `Enter` focuses one, `Ctrl+D` shuts it down.
 *
 * Row shape (captain-friendly):
 *   - `name`: session title (rolling "what's the captain working on
 *     now" string derived from the latest user prompt, or the wire
 *     `session_info_update` title when the agent ships one). Falls
 *     through to captain-set name, profile id, and agent id when
 *     there's no title yet (fresh instance, no turn submitted).
 *   - `description`: profile id, adapter, model, phase, cwd, mode,
 *     queue / terminal counts. The pieces the short headline used to
 *     advertise now sit in the details strip so the row's primary
 *     identifier matches what the captain reads in the header.
 *   - `kind`: short instance-id slug (acts as a quiet handle in the row).
 *
 * Right pane: `InstancesPreview.vue` renders the headline + the last
 * two transcript turns so the captain can scan recent context without
 * focusing the instance first.
 */

import InstancesPreview from './InstancesPreview.vue'
import { ToastTone } from '@components'
import { type PaletteEntry, PaletteMode, type PaletteSpec, usePalette, useActiveInstance, type InstanceId } from '@composables'
import { usePhase, useQueue, useSessionInfo, useTerminals, pushToast } from '@composables'
import { invoke, TauriCommand, type InstanceListEntry } from '@ipc'
import { log } from '@lib'

interface InstanceRow extends PaletteEntry {
  raw: InstanceListEntry
}

function rowFor(entry: InstanceListEntry, activeInstanceId: string | undefined): InstanceRow {
  const { info } = useSessionInfo(entry.instanceId)
  const { items } = useQueue(entry.instanceId)
  const { all: terminals } = useTerminals(entry.instanceId)
  const { phase } = usePhase(entry.instanceId)

  // Headline name: session title (set by either the wire
  // `session_info_update` or the latest user prompt) → captain-set
  // name → profile id → adapter id. Title beats the rest because the
  // captain just read that string in the header and on the chat
  // title chip — matching it in the picker means the row reads in
  // their own vocabulary.
  const headline = info.value.title ?? entry.name ?? entry.profileId ?? entry.agentId

  // Description groups: profile id first (the most-specific config
  // bundle the captain authored), then adapter / model / phase /
  // cwd / mode and live-state counts. Fuzzy filter still hits every
  // signal so a captain typing the profile id finds the row even
  // when the title is captioning the headline slot. `q<N>` / `t<N>`
  // only appear when non-zero so quiet rows stay clean.
  const meta: string[] = []

  if (entry.profileId) {
    meta.push(entry.profileId)
  }
  meta.push(entry.agentId)
  const model = info.value.model

  if (model) {
    meta.push(model)
  }
  meta.push(phase.value)
  const cwd = info.value.cwd

  if (cwd) {
    // Already display-formatted server-side (`tools::path::display_cwd`)
    // — chrome's CSS `text-overflow: ellipsis` handles overflow at
    // row width.
    meta.push(cwd)
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

  if (entry.instanceId === activeInstanceId) {
    meta.unshift('active')
  }

  return {
    id: entry.instanceId,
    name: headline,
    description: meta.join(' · '),
    kind: entry.instanceId.slice(0, 8),
    active: entry.instanceId === activeInstanceId,
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
    await invoke(TauriCommand.InstancesFocus, { instanceId: id })
  } catch(err) {
    log.error('invoke failed', { command: TauriCommand.InstancesFocus, id }, err)
    pushToast(ToastTone.Err, `instances focus failed: ${String(err)}`)
  }
}

export async function shutdownInstance(id: InstanceId): Promise<void> {
  try {
    await invoke(TauriCommand.InstancesShutdown, { instanceId: id })
  } catch(err) {
    log.error('invoke failed', { command: TauriCommand.InstancesShutdown, id }, err)
    pushToast(ToastTone.Err, `instances shutdown failed: ${String(err)}`)
  }
}

export async function openInstancesLeaf(): Promise<void> {
  const palette = usePalette()
  const { id: activeId } = useActiveInstance()
  const activeInstanceId = activeId.value

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

  const entries: PaletteEntry[] = instances.map((i) => rowFor(i, activeInstanceId))
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
      update(next.map((i) => rowFor(i, activeInstanceId)))
      // Preview's a separate component instance bound via spec.preview;
      // its data lands through `props` which already reads from
      // `instances` via the InstancesPreview component. Re-binding
      // here means the preview-pane updates alongside the row list.
      spec.preview.props.items = next
    }
  } satisfies PaletteSpec

  palette.open(spec)
}
