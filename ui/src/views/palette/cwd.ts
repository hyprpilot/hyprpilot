/**
 * cwd action — pops the native folder picker and commits the chosen
 * directory as the new cwd. Wired to both the root-palette `cwd`
 * entry and the header row-2 cwd-pill click; both paths converge
 * on `pickCwd()` so there's exactly one user-facing flow.
 *
 * Picker returns an absolute path → straight into `instance_restart`
 * with `ensure: true`. No typed-input fallback, no recent-cwd MRU,
 * no `paths_resolve` round-trip — the picker is the single way in.
 *
 * Browser / remote frontends don't have the Tauri dialog plugin AND
 * couldn't usefully pick a folder on the captain's phone for a daemon
 * running on a different machine (the absolute path wouldn't resolve
 * daemon-side anyway). Surface a toast and bail.
 */

import { ToastTone } from '@components'
import { useActiveInstance, useProfiles, useSessionInfo, useToasts } from '@composables'
import { invoke, TauriCommand } from '@ipc'
import { isRemoteHost } from '@ipc/remote-bridge'
import { log } from '@lib'

export async function pickCwd(): Promise<void> {
  const toasts = useToasts()

  if (isRemoteHost()) {
    toasts.push(ToastTone.Warn, 'cwd switching is available on the desktop overlay only')

    return
  }

  const { info: sessionInfo } = useSessionInfo()
  const { id: activeId } = useActiveInstance()
  const { profiles, selected: selectedProfile } = useProfiles()

  let absolute: string

  try {
    const { open: openDialog } = await import('@tauri-apps/plugin-dialog')
    const picked = await openDialog({
      directory: true,
      multiple: false,
      defaultPath: sessionInfo.value.cwd,
      title: 'Pick a working directory'
    })

    if (typeof picked !== 'string' || picked.length === 0) {
      // Captain cancelled; nothing to commit.
      return
    }
    absolute = picked
  } catch(err) {
    log.warn('palette-cwd: native folder picker failed', { err: String(err) })
    toasts.push(ToastTone.Err, `cwd picker failed: ${String(err)}`)

    return
  }

  // ensure=true: when no live actor matches `instanceId` (or none is
  // set), the daemon resolves `(agentId, profileId)` and bootstraps a
  // fresh actor rooted at `cwd`. Mirrors the models / modes / effort
  // leaves' empty-instance handling.
  const profileId = selectedProfile.value
  const agentId = profileId ? profiles.value.find((p) => p.id === profileId)?.agent : undefined

  try {
    await invoke(TauriCommand.InstanceRestart, {
      instanceId: activeId.value,
      cwd: absolute,
      ensure: true,
      agentId,
      profileId
    })
    toasts.push(ToastTone.Ok, `cwd → ${absolute}`)
  } catch(err) {
    log.warn('palette-cwd: instance_restart failed', { err: String(err) })
    toasts.push(ToastTone.Err, `cwd failed: ${String(err)}`)
  }
}
