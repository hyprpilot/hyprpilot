/**
 * Per-daemon-session cwd history. Module-scoped so every consumer of
 * `useCwdHistory()` shares the same MRU stack. Persisted to
 * localStorage (`hyprpilot.cwdHistory`) so a webview reload doesn't
 * wipe it; the `[skills] dirs`-style config-knob version is a
 * follow-up.
 *
 * Bounded length (`MAX_HISTORY = 10`); the oldest entry drops first.
 * `pushCwd()` deduplicates by exact-string equality and re-promotes
 * an existing match to the front so the MRU shape stays intact.
 */

import { ref, type Ref } from 'vue'

const STORAGE_KEY = 'hyprpilot.cwdHistory'
const MAX_HISTORY = 10

function safeLoad(): string[] {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY)
    if (!raw) {
      return []
    }
    const parsed = JSON.parse(raw) as unknown
    if (!Array.isArray(parsed)) {
      return []
    }

    return parsed.filter((entry): entry is string => typeof entry === 'string').slice(0, MAX_HISTORY)
  } catch {
    return []
  }
}

function safeSave(entries: string[]): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(entries))
  } catch {
    // localStorage unavailable (private mode, quota) — history stays in-memory only.
  }
}

const history = ref<string[]>(safeLoad())

export function pushCwd(cwd: string): void {
  const trimmed = cwd.trim()
  if (!trimmed) {
    return
  }
  const next = [trimmed, ...history.value.filter((entry) => entry !== trimmed)].slice(0, MAX_HISTORY)
  history.value = next
  safeSave(next)
}

export function clearCwdHistory(): void {
  history.value = []
  safeSave([])
}

export function useCwdHistory(): {
  history: Ref<string[]>
  push: typeof pushCwd
  clear: typeof clearCwdHistory
} {
  return { history, push: pushCwd, clear: clearCwdHistory }
}

/** Test-only: reset module state + clear localStorage. */
export function __resetCwdHistoryForTests(): void {
  history.value = []
  try {
    window.localStorage.removeItem(STORAGE_KEY)
  } catch {
    // ignore — test env may not have localStorage
  }
}
