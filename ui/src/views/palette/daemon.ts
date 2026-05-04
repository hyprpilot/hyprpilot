/**
 * Daemon palette leaf — captain-facing surface for the wire-side
 * `daemon/*` + `diag/*` RPCs. Each row dispatches a known method
 * through the `daemon_rpc` Tauri bridge (which routes through the
 * same `RpcDispatcher` the unix socket uses, so palette + `ctl`
 * reach identical handlers).
 *
 * Read-only methods (status / version / diag-snapshot) toast the
 * full response so the captain can eyeball the snapshot without
 * leaving the overlay. Mutating methods (reload / shutdown /
 * window-toggle) toast a brief confirmation.
 */
import { ToastTone } from '@components'
import { type PaletteEntry, PaletteMode, pushToast, usePalette } from '@composables'
import { invoke, TauriCommand } from '@ipc'
import { log } from '@lib'

interface DaemonAction {
  id: string
  name: string
  description: string
  method: string
  params?: unknown
  /** Tone for the success toast — `warn` for state-mutating methods. */
  successTone?: ToastTone
}

const ACTIONS: DaemonAction[] = [
  {
    id: 'daemon-reload',
    name: 'reload',
    description: 're-read config + skills + mcps.',
    method: 'daemon/reload',
    successTone: ToastTone.Warn
  },
  {
    id: 'daemon-shutdown',
    name: 'shutdown',
    description: 'graceful exit.',
    method: 'daemon/shutdown',
    successTone: ToastTone.Warn
  }
]

async function dispatchAction(action: DaemonAction): Promise<void> {
  try {
    const result = await invoke(TauriCommand.DaemonRpc, {
      method: action.method,
      params: action.params
    })

    log.info('daemon-rpc ok', { method: action.method, result })
    const tone = action.successTone ?? ToastTone.Ok
    const body = typeof result === 'object' && result !== null ? `${action.name} → ${JSON.stringify(result)}` : `${action.name} → ${String(result)}`

    pushToast(tone, body)
  } catch(err) {
    log.warn('daemon-rpc failed', { method: action.method, err: String(err) })
    pushToast(ToastTone.Err, `${action.name} failed: ${String(err)}`)
  }
}

export function openDaemonLeaf(): void {
  const { open } = usePalette()
  const entries: PaletteEntry[] = ACTIONS.map((a) => ({
    id: a.id,
    name: a.name,
    description: a.description
  }))

  open({
    mode: PaletteMode.Select,
    title: 'daemon',
    entries,
    onCommit(picks) {
      const pick = picks[0]

      if (!pick) {
        return
      }
      const action = ACTIONS.find((a) => a.id === pick.id)

      if (!action) {
        return
      }
      void dispatchAction(action)
    }
  })
}
