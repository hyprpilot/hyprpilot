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

import { type QueryClient } from '@tanstack/vue-query'

import { useActiveInstance } from './use-active-instance'
import { setDaemonCwd, setHomeDir } from './use-home-dir'
import { applyKeymapsFromObject } from './use-keymaps'
import { applyThemeFromObject } from './use-theme'
import { applyWindowStateFromObject } from './use-window'
import { applyCompletionConfigFromObject } from '../composer/use-completion'
import { prefetchInstanceChatFirstPage, prefetchInstanceMeta } from '../instance/use-focus-prefetch'
import {
  pushCurrentModeUpdate,
  setInstanceAgent,
  setInstanceName,
  setInstanceProfile
} from '../instance/use-session-info'
import { invoke, TauriCommand } from '@ipc'
import { log } from '@lib'

export async function applyBootSnapshot(queryClient?: QueryClient): Promise<boolean> {
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

  // Seed per-instance session-info from the registry list so the
  // header pills (agent / profile / mode / name) paint correctly the
  // moment Overlay.vue mounts. Without this, the captain on a remote
  // sees an empty header until brim-sync's later `instances/list`
  // round-trip lands — the very lag the boot snapshot exists to kill.
  for (const entry of snap.instances.instances) {
    if (entry.agentId) {
      setInstanceAgent(entry.instanceId, entry.agentId)
    }

    if (entry.profileId !== undefined) {
      setInstanceProfile(entry.instanceId, entry.profileId)
    }

    if (entry.mode !== undefined) {
      pushCurrentModeUpdate(entry.instanceId, { currentModeId: entry.mode })
    }

    if (entry.name !== undefined && entry.name.length > 0) {
      setInstanceName(entry.instanceId, entry.name)
    }
  }

  // Land the daemon's focused id on `useActiveInstance` immediately —
  // a remote that connects mid-session expects to see the same
  // instance the desktop is on, not "no active instance" empty.
  // `setIfUnset` preserves a captain's prior local choice on
  // subsequent re-syncs (focus event flipped to a different instance).
  if (snap.instances.focusedId) {
    useActiveInstance().setIfUnset(snap.instances.focusedId)

    // Kick off meta + chat-first-page prefetch in parallel with the
    // app mount so the chat viewport doesn't paint an empty state
    // before its `useInfiniteQuery` resolves. TanStack dedupes on
    // queryKey — Overlay.vue's brim-sync still runs but rides the
    // already-cached entries / in-flight requests rather than burning
    // fresh RTTs. Fire-and-forget; failures get caught by the
    // composables' own error paths.
    if (queryClient) {
      void prefetchInstanceMeta(queryClient, snap.instances.focusedId).catch((err: unknown) => {
        log.warn('boot-snapshot: focused meta prefetch failed', undefined, err)
      })
      void prefetchInstanceChatFirstPage(queryClient, snap.instances.focusedId).catch((err: unknown) => {
        log.warn('boot-snapshot: focused chat prefetch failed', undefined, err)
      })
    }
  }

  return true
}
