/**
 * Modes palette leaf — single-select picker over the active
 * instance's advertised mode list (claude-code's `plan` / `edit`,
 * codex's approval modes, etc.). Re-fetches from the daemon's
 * `instance_meta` command on every open instead of reading a
 * UI-side cache. The daemon's per-instance Arc<RwLock> holds the
 * authoritative state (refreshed on session/new, session/load,
 * set_mode, set_model, every turn-end), so this guarantees the
 * picker shows whatever the agent advertised most recently.
 *
 * On commit, fires `modes_set` Tauri command. Adapter stubs past
 * the membership check with a `-32603` error tied to K-251 — the
 * toast surfaces the error verbatim and the leaf lights up
 * automatically when K-251 lands.
 */

import { useActiveInstance } from '@composables'
import { type PaletteEntry, PaletteMode, type PaletteSpec, usePalette } from '@composables'
import { ToastTone } from '@components'
import { invoke, TauriCommand } from '@ipc'
import { log } from '@lib'
import { pushToast } from '@composables'

const EMPTY_ROW_ID = '__no-modes__'
const PLACEHOLDER_ROW_ID = '__no-instance__'
const ERROR_ROW_ID = '__meta-fetch-failed__'

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

function errorSpec(err: string): PaletteSpec {
  return {
    mode: PaletteMode.Select,
    title: 'modes',
    entries: [
      {
        id: ERROR_ROW_ID,
        name: 'modes fetch failed',
        description: err
      }
    ],
    onCommit: () => {}
  }
}

export async function openModesLeaf(): Promise<void> {
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
    log.warn('instance_meta failed (modes leaf)', { instanceId, err: message })
    open(errorSpec(message))

    return
  }

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
  const active = options.find((m) => m.id === snapshot.currentModeId)
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
