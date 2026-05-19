/**
 * Boot snapshot — one IPC call returns every chrome / config field
 * the loading screen needs before mount drops.
 *
 * Replaces five sequential `await invoke(...)` round-trips
 * (`get_theme` / `get_window_state` / `get_daemon_cwd` /
 * `get_keymaps` / `get_completion_config` + `agents_list` /
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

import { applyInstancesSnapshot, useActiveInstance } from './use-active-instance'
import { setDaemonCwd } from './use-daemon-cwd'
import { applyKeymapsFromObject } from './use-keymaps'
import { applyThemeFromObject } from './use-theme'
import { applyWindowStateFromObject } from './use-window'
import { applyCompletionConfigFromObject } from '../composer/use-completion'
import { prefetchInstanceMeta } from '../instance/use-focus-prefetch'
import { applyQueueChanged } from '../instance/use-queue'
import { pushCurrentModeUpdate, setInstanceAgent, setInstanceName, setInstanceProfile } from '../instance/use-session-info'
import { applyBootProfiles } from '../ui-state/use-profiles'
import { invoke, TauriCommand, type ChatSnapshot } from '@ipc'
import { log } from '@lib'

/**
 * Seed the TanStack chat cache from the boot snapshot's per-instance
 * `chats` map. Writes one `InfiniteData` shape per instance — same
 * key (`['snapshot-chat', instanceId]`) the `useInstanceChatInfiniteQuery`
 * mount reads from, same single-head-page shape
 * `prefetchInstanceChatFirstPage` produces. ChatViewport mounts
 * synchronously off the seeded cache.
 */
function seedChatCacheFromBoot(queryClient: QueryClient, chats: Record<string, ChatSnapshot>): void {
  for (const [instanceId, snap] of Object.entries(chats)) {
    queryClient.setQueryData(['snapshot-chat', instanceId], {
      pages: [snap],
      pageParams: [undefined as number | undefined]
    })
  }
}

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
  setDaemonCwd(snap.daemonCwd)
  // Seed the profiles singleton from the boot snapshot. The daemon's
  // `[[profiles]]` registry + the runtime-selected default id land
  // in one shot so the header pill paints correctly on first frame.
  // Live changes flow through the `acp:profile-changed` event the
  // singleton subscribes to on first `useProfiles()` call.
  applyBootProfiles(snap.profiles.profiles, snap.selectedProfileId)

  // Seed per-instance session-info from the registry list so the
  // header pills (agent / profile / mode / name) paint correctly the
  // moment Overlay.vue mounts. Without this, the captain on a remote
  // sees an empty header until brim-sync's later `instances/list`
  // round-trip lands — the very lag the boot snapshot exists to kill.
  //
  // Use `!= null` (covers null AND undefined) — older daemons (and
  // a buggy build of `boot_snapshot` before the typed wire shape
  // landed) ship `null` for the optional fields. Without this loose
  // check, `null !== undefined` slipped through to `null.length` and
  // threw, taking the whole boot pipeline with it (markBootDone never
  // fired, captain stuck on the loading screen).
  // Seed the per-process instance registry BEFORE the live router
  // subscribes — a remote captain on a fresh page load needs the
  // current membership + count so the row-1 instances button paints
  // correctly. Without this seed, `useActiveInstance().count` stays
  // 0 until the daemon broadcasts a spawn / shutdown event, which
  // may not happen for the entire mobile session.
  applyInstancesSnapshot(snap.instances.instances.map((entry) => entry.instanceId))

  for (const entry of snap.instances.instances) {
    if (entry.agentId) {
      setInstanceAgent(entry.instanceId, entry.agentId)
    }

    if (entry.profileId != null) {
      setInstanceProfile(entry.instanceId, entry.profileId)
    }

    if (entry.mode != null) {
      pushCurrentModeUpdate(entry.instanceId, { currentModeId: entry.mode })
    }

    if (entry.name != null && entry.name.length > 0) {
      setInstanceName(entry.instanceId, entry.name)
    }
  }

  // Seed the per-instance queue mirror so the QueueStrip + the
  // palette's `q<N>` badge render correctly on first paint without
  // an extra per-instance `instance/snapshot/queue` round-trip. The
  // daemon emits an empty `[]` for instances with no queued items
  // so absence here means "no instance" not "queue unknown".
  if (snap.queues) {
    for (const [instanceId, items] of Object.entries(snap.queues)) {
      applyQueueChanged(instanceId, items)
    }
  }

  // Seed the per-instance chat cache for EVERY live instance — not
  // just the focused one. The daemon ships a first chat-page snapshot
  // per instance in the boot payload (see `BOOT_CHAT_PAGE_LIMIT` in
  // `src-tauri/src/daemon/mod.rs`); writing each one directly into
  // the TanStack cache means the captain navigating into ANY visible
  // instance sees full history on the first frame. Closes the
  // "I only see the latest message" hydration gap that hit when:
  //   - the daemon had no `focusedId` (no chat prefetch fired)
  //   - the captain switched between instances (every flip triggered
  //     a fresh viewport-sized fetch instead of cache reuse)
  //   - a session_load replay raced the cache initialisation (the
  //     transcript-patcher's no-cache guard dropped the replay events
  //     because no pages existed yet).
  if (queryClient && snap.chats) {
    seedChatCacheFromBoot(queryClient, snap.chats)
  }

  // Land the daemon's focused id on `useActiveInstance` immediately —
  // a remote that connects mid-session expects to see the same
  // instance the desktop is on, not "no active instance" empty.
  // `setIfUnset` preserves a captain's prior local choice on
  // subsequent re-syncs (focus event flipped to a different instance).
  if (snap.instances.focusedId) {
    useActiveInstance().setIfUnset(snap.instances.focusedId)

    // Meta still rides on the per-instance round-trip — it's a small
    // payload and the `acp:instance-meta` event would refresh it
    // anyway on the next state change. Chat is already in the cache
    // (seeded from `snap.chats` above) so no chat round-trip here.
    if (queryClient) {
      const result = await prefetchInstanceMeta(queryClient, snap.instances.focusedId).catch((err: unknown) => {
        log.warn('boot-snapshot: focused meta prefetch failed', undefined, err)
      })

      void result
    }
  }

  return true
}
