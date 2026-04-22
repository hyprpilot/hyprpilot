import { invoke as tauriInvoke } from '@tauri-apps/api/core'
import { listen as tauriListen, type EventCallback, type UnlistenFn } from '@tauri-apps/api/event'

export async function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  return tauriInvoke<T>(command, args)
}

export async function listen<T>(event: string, cb: EventCallback<T>): Promise<UnlistenFn> {
  return tauriListen<T>(event, cb)
}

export type { EventCallback, UnlistenFn }

/** Shape returned by the `config/profiles` wire method + `profiles_list` Tauri command. */
export interface ProfileSummary {
  id: string
  agent: string
  model?: string
  has_prompt: boolean
  is_default: boolean
}

/** ACP-native `SessionInfo` shape returned by the `session_list` Tauri command. */
export interface SessionSummary {
  sessionId: string
  cwd: string
  title?: string
  updatedAt?: string
}

export interface ListSessionsArgs {
  agentId: string
  profileId?: string
  cwd?: string
}

export interface LoadSessionArgs {
  agentId: string
  profileId?: string
  sessionId: string
}

/** Fetches the `[[profiles]]` registry. Errors propagate — `useAcpProfiles::refresh` surfaces them via `lastErr`. */
export async function getProfiles(): Promise<ProfileSummary[]> {
  const r = await invoke<{ profiles: ProfileSummary[] }>('profiles_list')

  return r.profiles
}

/** Lists resumable sessions from the agent. Backend = K-243 Tauri command `session_list`. */
export async function listSessions(args: ListSessionsArgs): Promise<SessionSummary[]> {
  const r = await invoke<{ sessions: SessionSummary[] }>('session_list', { ...args })

  return r.sessions
}

/**
 * Resumes a session. Replay events stream through the existing
 * `acp:transcript` event fanout, so this returns `void` — the
 * transcript composable picks up the historical chunks and the
 * UI renders them live.
 */
export async function loadSession(args: LoadSessionArgs): Promise<void> {
  await invoke<void>('session_load', { ...args })
}
