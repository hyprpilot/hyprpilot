/**
 * cwd palette leaf — Input mode. The captain types a path; the
 * daemon's `path` source streams directory completions back as
 * picker rows; Enter commits either a highlighted suggestion or the
 * typed value verbatim.
 *
 * Recent cwd history surfaces as the initial row set so an empty
 * input still has something useful — captains hop between a small
 * set of project roots day-to-day. As soon as they type, the row
 * set switches to live `path`-source autocomplete (no fuzzy
 * filtering, no other completion sources). Empty result + non-empty
 * query still commits on Enter so the captain isn't forced to
 * pick from a list.
 *
 * Validation is client-side only — the path must look absolute or
 * resolve relative to the active-instance cwd. Real existence-check
 * happens on the Rust side at restart time (`instance_restart`
 * returns `-32602` for non-existent dirs); we surface those failures
 * via a `ToastTone.Err` toast.
 *
 * The follow-up `~/.config/hyprpilot/recent-cwds.toml` config knob
 * is out of scope for this MR — see the K-266 issue for the spec.
 */

import { ToastTone } from '@components'
import { truncateCwd, useActiveInstance, useCwdHistory, useHomeDir, usePalette, useSessionInfo, useToasts, type PaletteEntry, PaletteMode } from '@composables'
import { invoke, TauriCommand } from '@ipc'
import { log } from '@lib'

const COMPLETION_ROW_PREFIX = 'cwd-complete:'
const RECENT_ROW_PREFIX = 'cwd-recent:'
const COMPLETION_DEBOUNCE_MS = 120

function expandTilde(path: string, home?: string): string {
  if (!home) {
    return path
  }

  if (path === '~') {
    return home
  }

  if (path.startsWith('~/')) {
    return `${home}${path.slice(1)}`
  }

  return path
}

/**
 * Resolve a captain-typed path to an absolute one. `~` expands against
 * `$HOME`; relative paths (`./foo`, `../bar`, bare `foo`) resolve
 * against `cwdBase` (the active instance's cwd). Already-absolute
 * paths pass through. Returns undefined when the input can't be
 * resolved (no cwdBase + relative input, or empty after trim).
 */
function resolveAbsolute(rawPath: string, home: string | undefined, cwdBase: string | undefined): string | undefined {
  const trimmed = rawPath.trim()

  if (!trimmed) {
    return undefined
  }

  if (trimmed.startsWith('/')) {
    return trimmed
  }

  if (trimmed === '~' || trimmed.startsWith('~/')) {
    return expandTilde(trimmed, home)
  }

  if (!cwdBase) {
    return undefined
  }
  const base = cwdBase.endsWith('/') ? cwdBase.slice(0, -1) : cwdBase

  if (trimmed.startsWith('./')) {
    return `${base}/${trimmed.slice(2)}`
  }

  if (trimmed === '.') {
    return base
  }

  return `${base}/${trimmed}`
}

function shortenForToast(path: string, home?: string): string {
  return truncateCwd(path, 40, home)
}

async function commitCwd(rawPath: string, cwdBase?: string): Promise<void> {
  const { id: activeId } = useActiveInstance()
  const { homeDir } = useHomeDir()
  const { push } = useCwdHistory()
  const toasts = useToasts()

  const expanded = resolveAbsolute(rawPath, homeDir.value, cwdBase)

  if (!expanded) {
    toasts.push(ToastTone.Warn, `cwd: '${rawPath.trim()}' could not be resolved`)

    return
  }
  const instanceId = activeId.value

  if (!instanceId) {
    toasts.push(ToastTone.Err, 'cwd: no active instance to restart')

    return
  }

  try {
    await invoke(TauriCommand.InstanceRestart, { instanceId, cwd: expanded })
    push(expanded)
    toasts.push(ToastTone.Ok, `cwd → ${shortenForToast(expanded, homeDir.value)}`)
  } catch(err) {
    log.warn('palette-cwd: instance_restart failed', { err: String(err) })
    toasts.push(ToastTone.Err, `cwd failed: ${String(err)}`)
  }
}

/**
 * Path autocomplete via `completion_query` restricted to the `path`
 * source. Captain types `~/proj/`, `./src/`, `/etc/`, or a bare
 * `src/`; the daemon walks the corresponding directory and returns
 * directory matches. Files are filtered out client-side since cwd
 * must be a directory.
 *
 * Bare names with no sigil (`src`, `tests/foo`) are coerced to `./`
 * before sending — the path source's `detect()` rejects mid-text
 * tokens without a sigil, so the cwd palette injects one. The
 * captain's typed-as-relative intent then resolves against
 * `cwdBase` (the active instance's cwd).
 */
async function fetchPathCompletions(query: string, cwdBase?: string): Promise<PaletteEntry[]> {
  const looksAbsolute = query.startsWith('/') || query.startsWith('~/') || query === '~'
  const looksRelative = query.startsWith('./') || query.startsWith('../')
  const text = looksAbsolute || looksRelative ? query : `./${query}`
  const cursor = text.length

  try {
    const r = await invoke(TauriCommand.CompletionQuery, {
      text,
      cursor,
      cwd: cwdBase,
      manual: false,
      // Restrict the daemon walk to the `path` source — cwd palette
      // never wants skills / commands / ripgrep matches even when
      // the typed query happens to look like one of their sigils.
      sources: ['path']
    })

    if (!r || !Array.isArray(r.items)) {
      return []
    }

    return (
      r.items
        // Path source emits `detail: "dir"` for directories — cwd must
        // be a directory so files are filtered out.
        .filter((item) => item.kind === 'path' && item.detail === 'dir')
        .slice(0, 30)
        .map((item) => ({
          id: `${COMPLETION_ROW_PREFIX}${item.replacement.text}`,
          name: item.label,
          description: item.replacement.text,
          kind: 'directory'
        }))
    )
  } catch(err) {
    log.debug('palette-cwd: completion failed', { err: String(err) })

    return []
  }
}

export function openCwdLeaf(): void {
  const { open } = usePalette()
  const { history } = useCwdHistory()
  const { homeDir } = useHomeDir()
  const { info: sessionInfo } = useSessionInfo()

  const recentEntries: PaletteEntry[] = history.value.map((cwd) => ({
    id: `${RECENT_ROW_PREFIX}${cwd}`,
    name: truncateCwd(cwd, 60, homeDir.value),
    description: cwd
  }))

  let debounceTimer: ReturnType<typeof setTimeout> | undefined

  open({
    mode: PaletteMode.Input,
    title: 'cwd',
    placeholder: 'absolute or relative path…',
    entries: recentEntries,
    // Server-pre-filtered: directory completions arrive already
    // pruned against the typed query. Skip the Fuse pass that would
    // re-filter basenames against the raw path the captain typed.
    filtered: true,
    onQueryChange(query, update) {
      if (debounceTimer !== undefined) {
        clearTimeout(debounceTimer)
      }
      const trimmed = query.trim()

      if (trimmed.length === 0) {
        // Empty query → restore the recent list. Empty list with no
        // recents is fine: Input mode hides the "no matches"
        // empty-state, captain just sees the bare input.
        update(recentEntries)

        return
      }
      debounceTimer = setTimeout(() => {
        void fetchPathCompletions(trimmed, sessionInfo.value.cwd).then((completions) => {
          update(completions)
        })
      }, COMPLETION_DEBOUNCE_MS)
    },
    onCommit(picks, query) {
      const base = sessionInfo.value.cwd
      const pick = picks[0]

      // Highlighted autocomplete row → use its replacement text.
      // Highlighted recent row → use its absolute path id suffix.
      // No highlighted row (empty list / unhovered) → commit the
      // captain's typed query verbatim, resolved against cwdBase.
      if (pick?.id.startsWith(COMPLETION_ROW_PREFIX)) {
        void commitCwd(pick.id.slice(COMPLETION_ROW_PREFIX.length), base)

        return
      }

      if (pick?.id.startsWith(RECENT_ROW_PREFIX)) {
        void commitCwd(pick.id.slice(RECENT_ROW_PREFIX.length), base)

        return
      }

      if (query) {
        void commitCwd(query, base)
      }
    }
  })
}
