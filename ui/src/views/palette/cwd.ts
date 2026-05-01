/**
 * cwd palette leaf — single-select. Two row groups:
 *
 * 1. **recent** — entries from `useCwdHistory()` (this daemon
 *    session's MRU stack, persisted to localStorage). Each row commits
 *    against its absolute path.
 * 2. **manual input** — a single sentinel row that reads the live
 *    palette query as the path. The user types an absolute path into
 *    the search box and Enter commits it.
 *
 * Validation is client-side only — the path must look absolute. Real
 * existence-checks happen on the Rust side at restart time
 * (`instance_restart` returns `-32602` for non-existent dirs); we
 * surface those failures via a `ToastTone.Err` toast.
 *
 * The follow-up `~/.config/hyprpilot/recent-cwds.toml` config knob is
 * out of scope for this MR — see the K-266 issue for the spec.
 */

import { ToastTone } from '@components'
import {
  truncateCwd,
  useActiveInstance,
  useCwdHistory,
  useHomeDir,
  usePalette,
  useToasts,
  type PaletteEntry,
  PaletteMode
} from '@composables'
import { invoke, TauriCommand } from '@ipc'
import { log } from '@lib'

const MANUAL_ROW_ID = 'cwd-manual'

function isLikelyAbsolute(path: string): boolean {
  return path.startsWith('/') || path.startsWith('~')
}

function expandTilde(path: string, home?: string): string {
  if (!home) {
    return path
  }
  if (path === '~') {
    return home
  }
  if (path.startsWith('~/')) {
    return `${home}${path.slice(1)}`
  }

  return path
}

function shortenForToast(path: string, home?: string): string {
  return truncateCwd(path, 40, home)
}

async function commitCwd(rawPath: string): Promise<void> {
  const { id: activeId } = useActiveInstance()
  const { homeDir } = useHomeDir()
  const { push } = useCwdHistory()
  const toasts = useToasts()

  const trimmed = rawPath.trim()
  if (!trimmed) {
    toasts.push(ToastTone.Warn, 'cwd: empty path')

    return
  }
  if (!isLikelyAbsolute(trimmed)) {
    toasts.push(ToastTone.Warn, `cwd: '${trimmed}' is not an absolute path`)

    return
  }
  const instanceId = activeId.value
  if (!instanceId) {
    toasts.push(ToastTone.Err, 'cwd: no active instance to restart')

    return
  }

  const expanded = expandTilde(trimmed, homeDir.value)

  try {
    await invoke(TauriCommand.InstanceRestart, { instanceId, cwd: expanded })
    push(expanded)
    toasts.push(ToastTone.Ok, `cwd → ${shortenForToast(expanded, homeDir.value)}`)
  } catch (err) {
    log.warn('palette-cwd: instance_restart failed', { err: String(err) })
    toasts.push(ToastTone.Err, `cwd failed: ${String(err)}`)
  }
}

export function openCwdLeaf(): void {
  const { open } = usePalette()
  const { history } = useCwdHistory()
  const { homeDir } = useHomeDir()

  const recentEntries: PaletteEntry[] = history.value.map((cwd) => ({
    id: `cwd-recent:${cwd}`,
    name: truncateCwd(cwd, 60, homeDir.value),
    description: cwd
  }))

  const entries: PaletteEntry[] = [
    ...recentEntries,
    {
      id: MANUAL_ROW_ID,
      name: 'type a path…',
      description: 'enter an absolute path; tilde expands to $HOME'
    }
  ]

  open({
    mode: PaletteMode.Select,
    title: 'cwd',
    entries,
    onCommit(picks, query) {
      const pick = picks[0]
      if (!pick) {
        return
      }
      if (pick.id === MANUAL_ROW_ID) {
        void commitCwd(query ?? '')

        return
      }
      if (pick.id.startsWith('cwd-recent:')) {
        void commitCwd(pick.id.slice('cwd-recent:'.length))

        return
      }
    }
  })
}
