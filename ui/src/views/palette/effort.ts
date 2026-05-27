/**
 * Effort palette leaf — single-select picker over the active
 * instance's adapter-advertised `effort` config option. The backend
 * normalizes adapter wire ids (for example Codex ACP's
 * `reasoning_effort`) onto this common category before they reach the
 * palette, then maps `effort` back to the adapter wire id when
 * committing.
 *
 * The palette is generic over any config-option category the agent
 * advertises (vendor extensions show up here too) — effort is the
 * first concrete leaf; future categories slot in by reusing
 * `openConfigOptionLeaf(categoryId)` with a different id.
 */

import { ToastTone } from '@components'
import { pushConfigOptionChange, pushConfigOptionsUpdate, useActiveInstance, useProfiles, useSessionInfo, pushToast } from '@composables'
import { type PaletteEntry, PaletteMode, type PaletteSpec, usePalette } from '@composables'
import { invoke, TauriCommand } from '@ipc'
import { log } from '@lib'

const EMPTY_ROW_ID = '__no-effort__'
const ERROR_ROW_ID = '__effort-fetch-failed__'

function noOptionsSpec(message: string): PaletteSpec {
  return {
    mode: PaletteMode.Select,
    title: 'effort',
    entries: [
      {
        id: EMPTY_ROW_ID,
        name: 'no effort levels available',
        description: message
      }
    ],
    onCommit: () => {}
  }
}

function errorSpec(err: string): PaletteSpec {
  return {
    mode: PaletteMode.Select,
    title: 'effort',
    entries: [
      {
        id: ERROR_ROW_ID,
        name: 'effort fetch failed',
        description: err
      }
    ],
    onCommit: () => {}
  }
}

/**
 * Open the effort picker for the active instance. Reads the
 * advertised category off `useSessionInfo().info.configOptions`
 * (populated by the `config_option_update` listener); falls back
 * to a "fetch from daemon" pass when the cache is empty (post-boot,
 * pre-first-prompt) by triggering an `instance_meta` ensure round-
 * trip — the daemon's response carries the latest config_options
 * via the per-instance Arc<RwLock>.
 */
export async function openEffortLeaf(): Promise<void> {
  return openConfigOptionLeaf('effort', 'effort')
}

async function openConfigOptionLeaf(categoryId: string, paletteTitle: string): Promise<void> {
  const { open } = usePalette()
  const { id } = useActiveInstance()
  const { profiles, selected } = useProfiles()
  const instanceId = id.value
  const profileId = selected.value
  const agentId = profileId ? profiles.value.find((p) => p.id === profileId)?.agent : undefined
  const { info } = useSessionInfo()

  // Round-trip the meta call so the daemon-side per-instance state
  // refreshes its `configOptions` field. Mirrors modes/models — UI-
  // side cache is best-effort; daemon's lock is authoritative.
  let snapshot

  try {
    snapshot = await invoke(TauriCommand.InstanceMeta, {
      instanceId,
      ensure: true,
      agentId,
      profileId
    })
  } catch(err) {
    const message = String(err)

    log.warn(`instance_meta failed (${categoryId} leaf)`, { instanceId, err: message })
    open(errorSpec(message))

    return
  }

  const snapshotOptions = snapshot.configOptions ?? []

  if (snapshot.instanceId && snapshot.configOptions !== undefined) {
    pushConfigOptionsUpdate(snapshot.instanceId, snapshotOptions)
  }

  const categories = snapshotOptions.length > 0 ? snapshotOptions : info.value.configOptions
  const category = categories.find((c) => c.id === categoryId)

  if (!category) {
    open(noOptionsSpec(`no ${categoryId} category advertised yet — wait for the agent to push config_option_update`))

    return
  }

  if (category.options.length === 0) {
    open(noOptionsSpec(`${category.id}: agent advertised the category but no values yet`))

    return
  }

  const targetInstance = snapshot.instanceId ?? instanceId

  if (!targetInstance) {
    open(errorSpec('no instance id resolved after ensure'))

    return
  }

  const entries: PaletteEntry[] = category.options.map((opt) => ({
    id: opt.value,
    name: opt.name,
    description: opt.description,
    active: opt.value === category.currentValue
  }))
  const active = category.options.find((opt) => opt.value === category.currentValue)
  const preseed: PaletteEntry[] = active
    ? [
      {
        id: active.value,
        name: active.name,
        description: active.description
      }
    ]
    : []

  open({
    mode: PaletteMode.Select,
    title: paletteTitle,
    entries,
    preseedActive: preseed,
    async onCommit(picks) {
      const pick = picks[0]

      if (!pick) {
        return
      }
      const prev = active

      try {
        await invoke(TauriCommand.EffortSet, {
          instanceId: targetInstance,
          effortId: pick.id
        })
        pushToast(ToastTone.Ok, `${category.id} → ${pick.name}`)

        // Captain-initiated change → leave a chapter-break banner in
        // the transcript matching mode / model commits. pushConfigOptionChange
        // dedupes against the most-recent banner, so an agent echo via
        // `config_option_update` won't stack a second card.
        if (snapshot.sessionId) {
          pushConfigOptionChange(targetInstance, snapshot.sessionId, {
            categoryId: category.id,
            value: pick.id,
            name: pick.name,
            prevValue: prev?.value,
            prevName: prev?.name
          })
        }
      } catch(err) {
        const message = String(err)

        log.warn('config_option_set failed', {
          instanceId: targetInstance,
          categoryId: category.id,
          value: pick.id,
          err: message
        })
        pushToast(ToastTone.Err, `${category.id}: ${message}`)
      }
    }
  })
}
