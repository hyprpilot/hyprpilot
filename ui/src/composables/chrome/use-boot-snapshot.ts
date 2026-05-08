/**
 * Boot snapshot — one IPC call returns every chrome / config field
 * the loading screen needs before mount drops.
 *
 * Replaces six sequential `await invoke(...)` round-trips
 * (`get_theme` / `get_window_state` / `get_home_dir` / `get_daemon_cwd`
 * / `get_keymaps` / `get_completion_config` + `agents_list` /
 * `profiles_list` / `instances_list`) with one. The remote bridge
 * makes the round-trip cost particularly visible — every invoke rides
 * the same WS, so the captain spent up to 6× RTT staring at
 * "configuring window…" while the daemon already had every answer in
 * hand.
 *
 * Per-instance snapshot data (chat / terminals / per-instance meta)
 * stays on its own RPCs; brim-sync calls those after boot for the
 * focused instance only.
 *
 * Soft-fails when no Tauri host is bound (vitest jsdom / dev preview)
 * — caller decides whether to fall back to the granular loaders.
 */

import { setDaemonCwd, setHomeDir } from './use-home-dir'
import { applyKeymapsFromObject } from './use-keymaps'
import { applyThemeFromObject } from './use-theme'
import { applyWindowStateFromObject } from './use-window'
import { applyCompletionConfigFromObject } from '../composer/use-completion'
import { invoke, TauriCommand } from '@ipc'
import { log } from '@lib'

export async function applyBootSnapshot(): Promise<boolean> {
  let snap

  try {
    snap = await invoke(TauriCommand.BootSnapshot)
  } catch(err) {
    log.warn('boot_snapshot invoke failed; falling back to granular loaders', undefined, err)

    return false
  }

  applyThemeFromObject(snap.theme)
  applyWindowStateFromObject(snap.windowState)
  applyKeymapsFromObject(snap.keymaps)
  applyCompletionConfigFromObject(snap.completionConfig)
  setHomeDir(snap.homeDir)
  setDaemonCwd(snap.daemonCwd)

  return true
}
