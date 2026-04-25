/**
 * Models palette leaf — single-select picker over the active
 * instance's advertised model list. Reads from
 * `useSessionInfo().info.value.availableModels` (cached off
 * `current_model_update` / `session_info_update` envelopes); the
 * current selection is highlighted via the `preseedActive` slot.
 *
 * On commit, fires `models_set` Tauri command which dispatches
 * through `AcpAdapter::set_session_model`. Today the adapter
 * stubs past the membership check with a `-32603` error tied to
 * K-251; the toast surfaces the error verbatim. When K-251 lands
 * the leaf lights up automatically.
 */

import { useActiveInstance, useSessionInfo } from '@composables'
import { type PaletteEntry, PaletteMode, type PaletteSpec, usePalette } from '@composables/palette'
import { ToastTone } from '@components/types'
import { invoke, TauriCommand } from '@ipc'
import { log } from '@lib'
import { pushToast } from '@composables/use-toasts'

const EMPTY_ROW_ID = '__no-models__'
const PLACEHOLDER_ROW_ID = '__no-instance__'

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

export function openModelsLeaf(): void {
  const { open } = usePalette()
  const { id } = useActiveInstance()
  const instanceId = id.value
  if (!instanceId) {
    open(noInstanceSpec())

    return
  }

  const { info } = useSessionInfo(instanceId)
  const snapshot = info.value
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
  const active = options.find((m) => m.id === snapshot.model)
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
