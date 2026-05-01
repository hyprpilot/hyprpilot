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

import { useActiveInstance } from '@composables'
import { type PaletteEntry, PaletteMode, type PaletteSpec, usePalette } from '@composables'
import { ToastTone } from '@components'
import { invoke, TauriCommand } from '@ipc'
import { log } from '@lib'
import { pushToast } from '@composables'

const EMPTY_ROW_ID = '__no-models__'
const PLACEHOLDER_ROW_ID = '__no-instance__'
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

function noInstanceSpec(): PaletteSpec {
  return {
    mode: PaletteMode.Select,
    title: 'models',
    entries: [
      {
        id: PLACEHOLDER_ROW_ID,
        name: 'no active instance',
        description: 'submit a turn to spawn a session, then re-open the palette'
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
  const instanceId = id.value
  if (!instanceId) {
    open(noInstanceSpec())

    return
  }

  let snapshot
  try {
    snapshot = await invoke(TauriCommand.InstanceMeta, { instanceId })
  } catch (err) {
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
    description: m.description
  }))
  const active = options.find((m) => m.id === snapshot.currentModelId)
  const preseed: PaletteEntry[] = active
    ? [{ id: active.id, name: active.name, description: active.description }]
    : []

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
      try {
        await invoke(TauriCommand.ModelsSet, { instanceId, modelId: pick.id })
        pushToast(ToastTone.Ok, `model → ${pick.name}`)
      } catch (err) {
        const message = String(err)
        log.warn('models_set failed', { instanceId, modelId: pick.id, err: message })
        pushToast(ToastTone.Err, message)
      }
    }
  })
}
