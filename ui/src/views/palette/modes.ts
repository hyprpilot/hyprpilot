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

import { ToastTone } from '@components'
import { pushModeChange, useActiveInstance, useProfiles, pushToast } from '@composables'
import { type PaletteEntry, PaletteMode, type PaletteSpec, usePalette } from '@composables'
import { invoke, TauriCommand } from '@ipc'
import { log } from '@lib'

const EMPTY_ROW_ID = '__no-modes__'
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
  const { profiles, selected } = useProfiles()
  const instanceId = id.value
  const profileId = selected.value
  const agentId = profileId ? profiles.value.find((p) => p.id === profileId)?.agent : undefined

  let snapshot

  try {
    // ensure=true: when no live actor matches `instanceId` (or none
    // is set), the daemon resolves `(agentId, profileId)` and
    // bootstraps a fresh actor in-place. Picker populates against
    // the freshly-spawned instance instead of dead-ending with
    // "no active instance" on a clean overlay.
    snapshot = await invoke(TauriCommand.InstanceMeta, {
      instanceId,
      ensure: true,
      agentId,
      profileId
    })
  } catch(err) {
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
    description: m.description,
    active: m.id === snapshot.currentModeId
  }))
  const active = options.find((m) => m.id === snapshot.currentModeId)
  const preseed: PaletteEntry[] = active
    ? [
      {
        id: active.id,
        name: active.name,
        description: active.description
      }
    ]
    : []

  // Daemon echoes the resolved instance id when ensure-spawn ran —
  // route the commit there directly instead of awaiting the
  // registry's async auto-focus to refresh `useActiveInstance`.
  const targetInstance = snapshot.instanceId ?? instanceId

  if (!targetInstance) {
    open(errorSpec('no instance id resolved after ensure'))

    return
  }

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
      const prev = options.find((m) => m.id === snapshot.currentModeId)

      try {
        await invoke(TauriCommand.ModesSet, { instanceId: targetInstance, modeId: pick.id })
        pushToast(ToastTone.Ok, `mode → ${pick.name}`)

        // Captain-initiated change → leave a chapter-break banner in
        // the transcript matching the agent-emitted current_mode_update
        // path. pushModeChange dedupes against the most-recent banner,
        // so an agent echo (some adapters re-emit after set_mode) won't
        // stack a second card. Session id needed for the dedupe key
        // grouping; reach for the live one (snapshot has it).
        if (snapshot.sessionId) {
          pushModeChange(targetInstance, snapshot.sessionId, {
            modeId: pick.id,
            name: pick.name,
            prevModeId: prev?.id,
            prevName: prev?.name
          })
        }
      } catch(err) {
        const message = String(err)

        log.warn('modes_set failed', {
          instanceId: targetInstance,
          modeId: pick.id,
          err: message
        })
        pushToast(ToastTone.Err, message)
      }
    }
  })
}
