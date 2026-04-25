/**
 * Modes palette leaf — single-select picker over the active
 * instance's advertised mode list (claude-code's `plan` / `edit`,
 * codex's approval modes, etc.). Reads from
 * `useSessionInfo().info.value.availableModes`; the current
 * selection is highlighted via the `preseedActive` slot.
 *
 * On commit, fires `modes_set` Tauri command. Adapter stubs past
 * the membership check with a `-32603` error tied to K-251 — the
 * toast surfaces the error verbatim and the leaf lights up
 * automatically when K-251 lands.
 */

import { useActiveInstance, useSessionInfo } from '@composables'
import { type PaletteEntry, PaletteMode, type PaletteSpec, usePalette } from '@composables/palette'
import { ToastTone } from '@components/types'
import { invoke, TauriCommand } from '@ipc'
import { log } from '@lib'
import { pushToast } from '@composables/use-toasts'

const EMPTY_ROW_ID = '__no-modes__'
const PLACEHOLDER_ROW_ID = '__no-instance__'

function noOptionsSpec(message: string): PaletteSpec {
  return {
    mode: PaletteMode.Select,
    title: 'modes',
    entries: [
      {
        id: EMPTY_ROW_ID,
        name: 'no modes available',
        description: message
      }
    ],
    onCommit: () => {}
  }
}

function noInstanceSpec(): PaletteSpec {
  return {
    mode: PaletteMode.Select,
    title: 'modes',
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

export function openModesLeaf(): void {
  const { open } = usePalette()
  const { id } = useActiveInstance()
  const instanceId = id.value
  if (!instanceId) {
    open(noInstanceSpec())

    return
  }

  const { info } = useSessionInfo(instanceId)
  const snapshot = info.value
  const options = snapshot.availableModes
  if (options.length === 0) {
    open(noOptionsSpec('no current_mode_update advertised yet for this instance'))

    return
  }

  const entries: PaletteEntry[] = options.map((m) => ({
    id: m.id,
    name: m.name,
    description: m.description
  }))
  const active = options.find((m) => m.id === snapshot.mode)
  const preseed: PaletteEntry[] = active
    ? [{ id: active.id, name: active.name, description: active.description }]
    : []

  open({
    mode: PaletteMode.Select,
    title: 'modes',
    entries,
    preseedActive: preseed,
    async onCommit(picks) {
      const pick = picks[0]
      if (!pick) {
        return
      }
      try {
        await invoke(TauriCommand.ModesSet, { instanceId, modeId: pick.id })
        pushToast(ToastTone.Ok, `mode → ${pick.name}`)
      } catch (err) {
        const message = String(err)
        log.warn('modes_set failed', { instanceId, modeId: pick.id, err: message })
        pushToast(ToastTone.Err, message)
      }
    }
  })
}
