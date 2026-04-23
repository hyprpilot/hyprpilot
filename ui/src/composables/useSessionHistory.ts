import { onBeforeUnmount, onMounted, ref, watch, type Ref } from 'vue'

import { listen, listSessions, loadSession, type SessionSummary, type UnlistenFn } from '@ipc'

import { SessionState, type SessionStateEvent } from './useAdapter'

/**
 * Reactive wrapper around `session_list` + `session_load`. Both are
 * Tauri commands — a live ACP adapter is required on the daemon
 * side. Session entries come straight from the agent (ACP
 * `SessionInfo` shape). `load` triggers a resume; replay events
 * arrive through the existing `acp:transcript` event stream.
 */
export function useSessionHistory(agentId: Ref<string | undefined>, profileId: Ref<string | undefined>) {
  const sessions = ref<SessionSummary[]>([])
  const loading = ref(false)
  const lastErr = ref<string>()

  const unlisteners: UnlistenFn[] = []

  async function refresh(): Promise<void> {
    const agent = agentId.value
    if (!agent) {
      sessions.value = []

      return
    }
    loading.value = true
    lastErr.value = undefined
    try {
      sessions.value = await listSessions({ agentId: agent, profileId: profileId.value })
    } catch (err) {
      lastErr.value = String(err)
      sessions.value = []
    } finally {
      loading.value = false
    }
  }

  async function load(sessionId: string): Promise<void> {
    const agent = agentId.value
    if (!agent) {
      return
    }
    try {
      await loadSession({ agentId: agent, profileId: profileId.value, sessionId })
    } catch (err) {
      lastErr.value = String(err)
    }
  }

  onMounted(async () => {
    unlisteners.push(
      await listen<SessionStateEvent>('acp:instance-state', (e) => {
        if (e.payload.state === SessionState.Ended) {
          void refresh()
        }
      })
    )
    void refresh()
  })

  onBeforeUnmount(() => {
    for (const u of unlisteners) u()
    unlisteners.length = 0
  })

  watch([agentId, profileId], () => {
    void refresh()
  })

  return {
    sessions,
    loading,
    lastErr,
    refresh,
    load
  }
}
