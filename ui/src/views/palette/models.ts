/**
 * Models palette leaf — single-select picker over the active
 * instance's advertised model list. Re-fetches from the daemon's
 * `instance_meta` command on every open instead of reading a
 * UI-side cache. The daemon's per-instance Arc<RwLock> holds the
 * authoritative state, refreshed on every session/new, session/load,
 * set_mode, set_model, and turn-end.
 *
 * On commit, fires `models_set` Tauri command which dispatches
 * through `AcpAdapter::set_session_model`. Today the adapter
 * stubs past the membership check with a `-32603` error tied to
 * K-251; the toast surfaces the error verbatim. When K-251 lands
 * the leaf lights up automatically.
 */

import { ToastTone } from '@components'
import { pushModelChange, useActiveInstance, useProfiles, pushToast } from '@composables'
import { type PaletteEntry, PaletteMode, type PaletteSpec, usePalette } from '@composables'
import { invoke, TauriCommand } from '@ipc'
import { log } from '@lib'

const EMPTY_ROW_ID = '__no-models__'
const ERROR_ROW_ID = '__meta-fetch-failed__'

function noOptionsSpec(message: string): PaletteSpec {
  return {
    mode: PaletteMode.Select,
    title: 'models',
    entries: [
      {
        id: EMPTY_ROW_ID,
        name: 'no models available',
        description: message
      }
    ],
    onCommit: () => {}
  }
}

function errorSpec(err: string): PaletteSpec {
  return {
    mode: PaletteMode.Select,
    title: 'models',
    entries: [
      {
        id: ERROR_ROW_ID,
        name: 'models fetch failed',
        description: err
      }
    ],
    onCommit: () => {}
  }
}

export async function openModelsLeaf(): Promise<void> {
  const { open } = usePalette()
  const { id } = useActiveInstance()
  const { profiles, selected } = useProfiles()
  const instanceId = id.value
  const profileId = selected.value
  const agentId = profileId ? profiles.value.find((p) => p.id === profileId)?.agent : undefined

  let snapshot

  try {
    // ensure=true: when no live actor matches `instanceId` (or none
    // is set), the daemon resolves `(agentId, profileId)` and
    // bootstraps a fresh actor in-place. See modes.ts for the rationale.
    snapshot = await invoke(TauriCommand.InstanceMeta, {
      instanceId,
      ensure: true,
      agentId,
      profileId
    })
  } catch(err) {
    const message = String(err)

    log.warn('instance_meta failed (models leaf)', { instanceId, err: message })
    open(errorSpec(message))

    return
  }

  const options = snapshot.availableModels

  if (options.length === 0) {
    open(noOptionsSpec('no current_model_update advertised yet for this instance'))

    return
  }

  const entries: PaletteEntry[] = options.map((m) => ({
    id: m.id,
    name: m.name,
    description: m.description,
    active: m.id === snapshot.currentModelId
  }))
  const active = options.find((m) => m.id === snapshot.currentModelId)
  const preseed: PaletteEntry[] = active
    ? [
      {
        id: active.id,
        name: active.name,
        description: active.description
      }
    ]
    : []

  const targetInstance = snapshot.instanceId ?? instanceId

  if (!targetInstance) {
    open(errorSpec('no instance id resolved after ensure'))

    return
  }

  open({
    mode: PaletteMode.Select,
    title: 'models',
    entries,
    preseedActive: preseed,
    async onCommit(picks) {
      const pick = picks[0]

      if (!pick) {
        return
      }
      const prev = options.find((m) => m.id === snapshot.currentModelId)

      try {
        await invoke(TauriCommand.ModelsSet, { instanceId: targetInstance, modelId: pick.id })
        pushToast(ToastTone.Ok, `model → ${pick.name}`)

        // Same chapter-break treatment as the modes leaf. Agents
        // don't currently emit `current_model_update` echoes for
        // session/set_model, so this is the only banner source for
        // user-initiated model switches today.
        if (snapshot.sessionId) {
          pushModelChange(targetInstance, snapshot.sessionId, {
            modelId: pick.id,
            name: pick.name,
            prevModelId: prev?.id,
            prevName: prev?.name
          })
        }
      } catch(err) {
        const message = String(err)

        log.warn('models_set failed', {
          instanceId: targetInstance,
          modelId: pick.id,
          err: message
        })
        pushToast(ToastTone.Err, message)
      }
    }
  })
}
