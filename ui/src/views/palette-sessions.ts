/**
 * Sessions palette leaf — lists resumable sessions from the daemon
 * (`session_list` Tauri command, served by `acp::list_sessions`).
 *
 * - Enter on a row → `session_load { sessionId }` against a freshly
 *   minted instance UUID; the daemon adopts the supplied id verbatim.
 * - Ctrl+D → `sessions/forget` on the wire. The server-side handler
 *   panics with `unimplemented!()` today (ACP 0.12 has no
 *   session-delete verb); per CLAUDE.md "stubs panic, they don't
 *   pretend" the client entry point refuses here too rather than
 *   round-trip a panic. Surfaces a warn toast so the user sees why.
 *
 * Right-pane preview rides on `PaletteSpec.preview` — the palette
 * passes the currently highlighted entry to `PaletteSessionsPreview`,
 * which calls `sessions_info` with a 200ms debounce.
 */

import PaletteSessionsPreview from './PaletteSessionsPreview.vue'
import { type PaletteEntry, PaletteMode, type PaletteSpec, usePalette } from '@composables/palette'
import { setSessionRestored } from '@composables/use-session-info'
import { pushToast } from '@composables/use-toasts'
import { ToastTone } from '@components/types'
import { listSessions, loadSession, type SessionSummary } from '@ipc'
import { log } from '@lib'

interface SessionsLeafEntry extends PaletteEntry {
  sessionId: string
}

/** ISO-8601 → "5m ago" / "2h ago" / "3d ago". Returns the raw timestamp on parse failure. */
export function relativeFromNow(iso: string | undefined, now: () => number = Date.now): string {
  if (!iso) {
    return ''
  }
  const ts = Date.parse(iso)
  if (Number.isNaN(ts)) {
    return iso
  }
  const deltaSec = Math.max(0, Math.floor((now() - ts) / 1000))
  if (deltaSec < 60) {
    return `${deltaSec}s ago`
  }
  const min = Math.floor(deltaSec / 60)
  if (min < 60) {
    return `${min}m ago`
  }
  const hr = Math.floor(min / 60)
  if (hr < 24) {
    return `${hr}h ago`
  }
  const days = Math.floor(hr / 24)
  if (days < 30) {
    return `${days}d ago`
  }
  const months = Math.floor(days / 30)
  if (months < 12) {
    return `${months}mo ago`
  }

  return `${Math.floor(months / 12)}y ago`
}

function shortenCwd(raw: string): string {
  // Light shortening — the right pane shows the full path. Keep last
  // three segments and prepend an ellipsis if anything was dropped.
  const segments = raw.split('/').filter((s) => s.length > 0)
  if (segments.length <= 3) {
    return raw
  }

  return `…/${segments.slice(-3).join('/')}`
}

export function buildSessionEntries(sessions: SessionSummary[], now: () => number = Date.now): SessionsLeafEntry[] {
  return sessions.map((s) => {
    const name = s.title?.trim() ? s.title : s.sessionId
    const cwd = shortenCwd(s.cwd)
    const rel = relativeFromNow(s.updatedAt, now)
    const description = [cwd, rel].filter((part) => part.length > 0).join('  ·  ')

    return {
      id: s.sessionId,
      sessionId: s.sessionId,
      name,
      description
    }
  })
}

function buildSpec(title: string, entries: SessionsLeafEntry[]): PaletteSpec {
  return {
    mode: PaletteMode.Select,
    title,
    entries,
    preview: { component: PaletteSessionsPreview },
    onCommit(picks) {
      const pick = picks[0] as SessionsLeafEntry | undefined
      if (!pick) {
        return
      }
      // Mint the target instance up-front so the restored flag keys
      // off the resumed handle, not whatever happens to be active when
      // the await resolves. Mirrors `useSessionHistory.load`.
      const target = crypto.randomUUID()
      void loadSession({ sessionId: pick.sessionId, instanceId: target })
        .then(() => {
          setSessionRestored(target, true)
        })
        .catch((err) => {
          log.warn('palette-sessions: load failed', { err })
          pushToast(ToastTone.Err, `session load failed: ${String(err)}`)
        })
    },
    onDelete() {
      // Per CLAUDE.md "stubs panic, they don't pretend": the wire
      // `sessions/forget` panics today (ACP 0.12 has no session-delete
      // verb), so we don't round-trip — surface the gap here so the
      // user sees the reason rather than a daemon crash.
      pushToast(ToastTone.Warn, 'sessions/forget: not yet implemented (ACP 0.12 lacks a session-delete verb)')
      log.warn('palette-sessions: delete not yet implemented')
    }
  }
}

export async function openSessionsLeaf(): Promise<void> {
  const palette = usePalette()

  // Pop the palette open immediately with a "loading…" placeholder so
  // the click feels instant; the real list patches in once
  // `session_list` returns. Avoids the "click → wait → palette" stall.
  palette.open(buildSpec('sessions — loading…', []))

  try {
    const sessions = await listSessions({})
    const entries = buildSessionEntries(sessions)
    const title = entries.length === 0 ? 'sessions — empty' : 'sessions'
    palette.close()
    palette.open(buildSpec(title, entries))
  } catch (err) {
    log.warn('palette-sessions: list failed', { err })
    pushToast(ToastTone.Err, `sessions/list failed: ${String(err)}`)
    // Leave the placeholder open so the user can Esc out cleanly.
  }
}
