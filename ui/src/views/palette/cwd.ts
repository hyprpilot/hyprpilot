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
 * Path resolution (`~` expansion, `${VAR}` interpolation, relative→
 * absolute against the active-instance cwd) lives daemon-side via
 * `paths_resolve` for the relative-against-cwdBase pre-resolution
 * step (the daemon also re-normalizes at the actor boundary, so the
 * round-trip is purely so the captain's MRU history records a single
 * canonical form for ranking). The daemon ships pre-formatted display
 * strings (`~/proj/foo`) on every wire surface — the frontend renders
 * verbatim and never reaches for `$HOME`.
 */

import { ToastTone } from '@components'
import { useActiveInstance, useCwdHistory, usePalette, useProfiles, useSessionInfo, useToasts, type PaletteEntry, PaletteMode } from '@composables'
import { invoke, TauriCommand } from '@ipc'
import { log } from '@lib'

const COMPLETION_ROW_PREFIX = 'cwd-complete:'
const RECENT_ROW_PREFIX = 'cwd-recent:'
const COMPLETION_DEBOUNCE_MS = 120

async function commitCwd(rawPath: string, cwdBase?: string): Promise<void> {
  const { id: activeId } = useActiveInstance()
  const { profiles, selected } = useProfiles()
  const { push } = useCwdHistory()
  const toasts = useToasts()

  let absolute: string | null

  try {
    absolute = await invoke(TauriCommand.PathsResolve, { raw: rawPath, cwdBase })
  } catch(err) {
    log.warn('palette-cwd: paths_resolve failed', { err: String(err) })
    toasts.push(ToastTone.Err, `cwd: resolve failed: ${String(err)}`)

    return
  }

  if (!absolute) {
    toasts.push(ToastTone.Warn, `cwd: '${rawPath.trim()}' could not be resolved`)

    return
  }
  // ensure=true: when no live actor matches `instanceId` (or none is
  // set), the daemon resolves `(agentId, profileId)` and bootstraps a
  // fresh actor rooted at `cwd`. Mirrors the models / modes / effort
  // leaves' empty-instance handling.
  const profileId = selected.value
  const agentId = profileId ? profiles.value.find((p) => p.id === profileId)?.agent : undefined

  try {
    await invoke(TauriCommand.InstanceRestart, {
      instanceId: activeId.value,
      cwd: absolute,
      ensure: true,
      agentId,
      profileId
    })
    push(absolute)
    // The daemon will emit `acp:instance-meta` with the
    // display-formatted form once the new actor reaches Running;
    // the toast quotes the captain's typed input verbatim until then
    // so the feedback isn't blocked on a wire round-trip.
    toasts.push(ToastTone.Ok, `cwd → ${rawPath.trim()}`)
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

/**
 * Fuzzy-rank the captain's recent cwds via `completion/rank`. Empty
 * query → identity order (MRU). Non-empty query → daemon's nucleo
 * matcher ranks recents against the typed text. Same RPC any other
 * caller-supplied candidate list lands on; identical ranking
 * across UI / Neovim plugin / future frontends.
 */
async function rankRecents(query: string, recents: string[]): Promise<PaletteEntry[]> {
  if (recents.length === 0) {
    return []
  }
  // Recents are stored in absolute form (the byte-canonical key for
  // the MRU dedupe); render them in display form for the row label
  // by passing the absolute through the daemon's display helper. We
  // skip the round-trip here — the captain's own typed shape is
  // already canonical (`~/proj` → typed as `~/proj`), and absolutes
  // collapse only if they sit under `$HOME` (which the UI no longer
  // tracks). Showing the absolute is acceptable; the home prefix
  // collapse round-trip happens at next reload via the daemon-emitted
  // session.cwd.
  const candidates = recents.map((cwd) => ({
    id: cwd,
    label: cwd,
    description: cwd
  }))

  try {
    const r = await invoke(TauriCommand.CompletionRank, { query, candidates })

    if (!r || !Array.isArray(r.items)) {
      return []
    }

    return r.items.map((item) => ({
      id: `${RECENT_ROW_PREFIX}${item.replacement.text}`,
      name: item.label,
      description: item.detail ?? item.replacement.text,
      kind: 'recent'
    }))
  } catch(err) {
    log.debug('palette-cwd: rank failed', { err: String(err) })

    return []
  }
}

export function openCwdLeaf(): void {
  const { open } = usePalette()
  const { history } = useCwdHistory()
  const { info: sessionInfo } = useSessionInfo()

  // Initial state: recents in MRU order, projected synchronously
  // so first paint has rows. Empty-query rank is identity, so we
  // skip the round-trip — captain hasn't typed anything to fuzzy-
  // match against. Render the stored cwd verbatim; the daemon's
  // wire shape collapses `$HOME` already on subsequent emits, so
  // the moment the captain commits a recent + the agent reports
  // back, the row label updates to the display form on next open.
  const initialRecents: PaletteEntry[] = history.value.map((cwd) => ({
    id: `${RECENT_ROW_PREFIX}${cwd}`,
    name: cwd,
    description: cwd
  }))

  let debounceTimer: ReturnType<typeof setTimeout> | undefined

  open({
    mode: PaletteMode.Input,
    title: 'cwd',
    placeholder: 'absolute or relative path…',
    entries: initialRecents,
    // Server-pre-filtered: directory completions + recents arrive
    // already ranked against the typed query. Skip the generic
    // `completion/rank` pass that would re-rank basenames against
    // the raw path query.
    filtered: true,
    onQueryChange(query, update) {
      if (debounceTimer !== undefined) {
        clearTimeout(debounceTimer)
      }
      const trimmed = query.trim()

      if (trimmed.length === 0) {
        update(initialRecents)

        return
      }
      debounceTimer = setTimeout(() => {
        // Path completions (live disk walk) + recents (captain
        // history fuzzy-ranked against query) co-exist on the same
        // typed input. Captain typing `proj` sees both "proj/ in
        // cwd" (path source) and "/home/cenk/work/proj" from MRU
        // (recents source). Paths render first since they're more
        // immediately actionable.
        const recentsP = rankRecents(trimmed, history.value)
        const pathsP = fetchPathCompletions(trimmed, sessionInfo.value.cwd)

        void Promise.all([pathsP, recentsP]).then(([paths, recents]) => {
          update([...paths, ...recents])
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
