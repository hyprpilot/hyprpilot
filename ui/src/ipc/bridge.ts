import { invoke as tauriInvoke } from '@tauri-apps/api/core'
import { listen as tauriListen, type EventCallback, type UnlistenFn } from '@tauri-apps/api/event'

import { TauriCommand, TauriEvent, type TauriCommandResult, type TauriEventPayload } from './commands'
import type { ListSessionsArgs, LoadSessionArgs, ProfileSummary, SessionSummary } from './types'

export async function invoke<K extends TauriCommand>(
  command: K,
  args?: Record<string, unknown>
): Promise<TauriCommandResult[K]> {
  return tauriInvoke<TauriCommandResult[K]>(command, args)
}

export async function listen<K extends TauriEvent>(
  event: K,
  cb: EventCallback<TauriEventPayload[K]>
): Promise<UnlistenFn> {
  return tauriListen<TauriEventPayload[K]>(event, cb)
}

export type { EventCallback, UnlistenFn }

/** Fetches the `[[profiles]]` registry. Errors propagate — `useProfiles::refresh` surfaces them via `lastErr`. */
export async function getProfiles(): Promise<ProfileSummary[]> {
  const r = await invoke(TauriCommand.ProfilesList)

  return r.profiles
}

/** Lists resumable sessions from the agent. Backend = K-243 Tauri command `session_list`. */
export async function listSessions(args: ListSessionsArgs): Promise<SessionSummary[]> {
  const r = await invoke(TauriCommand.SessionList, { ...args })

  return r.sessions
}

/**
 * Resumes a session. Replay events stream through the existing
 * `acp:transcript` event fanout, so this returns `void` — the
 * transcript composable picks up the historical chunks and the
 * UI renders them live.
 */
export async function loadSession(args: LoadSessionArgs): Promise<void> {
  await invoke(TauriCommand.SessionLoad, { ...args })
}
