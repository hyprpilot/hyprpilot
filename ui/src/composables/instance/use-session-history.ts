import { useQueryClient } from '@tanstack/vue-query'
import { ref, watch, type Ref } from 'vue'

import { setSessionRestoring } from './use-session-info'
import { useActiveInstance } from '../chrome/use-active-instance'
import { pushToast } from '../ui-state/use-toasts'
import { ToastTone } from '@components'
import { invoke, TauriCommand, type SessionSummary } from '@ipc'
import { log } from '@lib'

// Cache window — every `refresh()` call inside this many ms returns
// the cached list without spawning another ephemeral list-only ACP
// actor. Each fresh listing currently boots a full agent subprocess
// (initialize → list → shutdown ≈ 500ms each, plus heavy CPU + bunx
// download cost for claude-agent-acp), so reactive re-fetches on
// every component mount / state-event were dominating the daemon log
// + chewing through process spawns. 30s strikes a balance between
// "list reflects fresh state when the user has just done something"
// and "don't respawn agents on every reactive tick".
const CACHE_TTL_MS = 30_000

/**
 * Reactive wrapper around `session_list` + `session_load`. Both are
 * Tauri commands — a live ACP adapter is required on the daemon
 * side. Session entries come straight from the agent (ACP
 * `SessionInfo` shape). `load` triggers a resume; replay events
 * arrive through the existing `acp:transcript` event stream.
 *
 * The cached `sessions` ref is shared across every call so multiple
 * mounts (Overlay + sessions palette + idle-screen preview) read
 * from one source. `refresh()` round-trips to the daemon at most
 * once per `CACHE_TTL_MS`; explicit `refresh({ force: true })`
 * bypasses the window when the caller knows state changed (after
 * a `loadSession` resume, after a manual delete, etc.).
 */
const sessions = ref<SessionSummary[]>([])
const loading = ref(false)
const lastErr = ref<string>()
let lastFetchAt = 0
let lastAgentId: string | undefined
let lastProfileId: string | undefined

export interface UseSessionHistoryApi {
  sessions: Ref<SessionSummary[]>
  loading: Ref<boolean>
  lastErr: Ref<string | undefined>
  refresh: (opts?: { force?: boolean }) => Promise<void>
  /// Resume a persisted session. `cwd` overrides the resolved
  /// profile's cwd at the daemon side — required for claude-agent-acp,
  /// which scopes sessions by cwd ("Resource not found" otherwise).
  /// UI consumers pass `session.cwd` from `session_list`.
  load: (sessionId: string, cwd?: string) => Promise<void>
}

export function useSessionHistory(agentId: Ref<string | undefined>, profileId: Ref<string | undefined>): UseSessionHistoryApi {
  const queryClient = useQueryClient()

  async function refresh(opts: { force?: boolean } = {}): Promise<void> {
    const agent = agentId.value

    if (!agent) {
      sessions.value = []

      return
    }
    const cacheStillValid = !opts.force && lastAgentId === agent && lastProfileId === profileId.value && Date.now() - lastFetchAt < CACHE_TTL_MS && sessions.value.length > 0

    if (cacheStillValid) {
      return
    }
    loading.value = true
    lastErr.value = undefined

    try {
      const r = await invoke(TauriCommand.SessionList, { agentId: agent, profileId: profileId.value })

      sessions.value = r.sessions
      lastFetchAt = Date.now()
      lastAgentId = agent
      lastProfileId = profileId.value
    } catch(err) {
      const message = String(err)

      lastErr.value = message
      sessions.value = []
      // Surface the failure as a toast so the user sees why their
      // sessions list went empty. Without this the picker / idle
      // landing both render "no sessions configured" with no
      // diagnostic of why the underlying `session_list` call
      // failed.
      pushToast(ToastTone.Err, `sessions list failed: ${message}`)
    } finally {
      loading.value = false
    }
  }

  async function load(sessionId: string, cwd?: string): Promise<void> {
    const agent = agentId.value

    if (!agent) {
      pushToast(ToastTone.Warn, 'no agent resolved — cannot restore session')
      log.warn('session-history: load called with no resolved agent', { sessionId })

      return
    }
    // Mint the target instance id up-front so the restored flag keys
    // off the resumed handle, not whatever happens to be active when
    // the await resolves. `session_load` adopts the supplied UUID
    // verbatim — no race window between the daemon minting one and
    // the renderer learning it.
    const target = crypto.randomUUID()

    log.info('session-history: loading session', {
      sessionId,
      target,
      agent,
      profile: profileId.value
    })
    // Flip the transient `restoring` flag BEFORE the round-trip so
    // the chat-transcript <Loading> overlay paints the moment the
    // user clicks. Cleared by use-session-stream on the first
    // TurnEnded for `target` (the daemon's auto-cancel after
    // session/load triggers one). Stays set on failure so the user
    // sees the spinner and the err toast simultaneously — clearing
    // is the success path.
    setSessionRestoring(target, true)

    // Seed the chat cache for `target` BEFORE the SessionLoad
    // round-trip. The daemon's session/load replay fires `acp:transcript`
    // events for every replayed item; `transcript-patcher`'s
    // "no cache → drop" guard would silently throw those events
    // away because TanStack hasn't seen the `target` key yet
    // (ChatViewport mounts later, after the focus event lands).
    // A single empty head page is enough for the patcher to find
    // the cache key + append the live items to `head.items`.
    queryClient.setQueryData(['snapshot-chat', target], {
      pages: [
        {
          items: [],
          oldestSeq: undefined,
          latestSeq: undefined,
          hasMore: false
        }
      ],
      pageParams: [undefined as number | undefined]
    })

    try {
      await invoke(TauriCommand.SessionLoad, {
        agentId: agent,
        profileId: profileId.value,
        sessionId,
        instanceId: target,
        cwd
      })
      pushToast(ToastTone.Ok, 'restoring session…')
    } catch(err) {
      const message = String(err)

      lastErr.value = message
      log.warn('session-history: loadSession failed', { sessionId, err: message })
      pushToast(ToastTone.Err, `restore failed: ${message}`)
      setSessionRestoring(target, false)
    }
  }

  // **Skip eager fetch when an active instance is set.** Each
  // `session_list` round-trip spawns an ephemeral `Bootstrap::ListOnly`
  // ACP actor — a bunx subprocess for `claude-agent-acp` (~430ms best
  // case, multi-second cold-start when the package isn't cached). On
  // a remote captain's first boot this was the dominant boot-blocker
  // visible in the HAR (~28s gap between `boot_snapshot` returning
  // and `acp:instance-state ended` for the ephemeral actor).
  //
  // The session list only feeds the idle-screen preview + the
  // sessions palette leaf. When the captain has an active instance
  // (boot snapshot now seeds this), the chat viewport renders
  // instead of the idle screen — fetching the list at boot is
  // wasted IPC + a wasted bunx spawn. Idle / palette consumers
  // explicitly call `refresh()` when they actually render.
  const { id: activeInstanceId } = useActiveInstance()

  watch(
    [agentId, profileId, activeInstanceId],
    () => {
      // Skip whenever an instance is active: the chat viewport is
      // rendering, not the idle screen. The `sessions` ref stays at
      // its prior state (empty on first boot, populated on later
      // (agent, profile) flips); consumers that need fresh data call
      // `refresh()` directly.
      if (activeInstanceId.value) {
        return
      }
      void refresh()
    },
    { immediate: true }
  )

  return {
    sessions,
    loading,
    lastErr,
    refresh,
    load
  }
}
