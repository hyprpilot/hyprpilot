import { computed, ref, type ComputedRef, type Ref } from 'vue'

import { type CompletionItem, type CompletionQueryResponse, type CompletionResolveResponse, type CompletionSourceId, invoke, TauriCommand } from '@ipc'
import { log } from '@lib'

/**
 * Composer autocomplete state machine — driven by daemon's
 * `completion/{query,resolve,cancel}` Tauri commands. UI tracks the
 * latest query / resolve ids as watermarks; older responses are
 * dropped on receipt without rendering.
 *
 * Lifecycle (CLAUDE.md plan):
 *   Closed → Opening → Open → Resolving → Committing → Closed
 *
 * Debounces:
 *   - Query: 30ms (avoid churn on fast typists, stay under
 *     visible-lag threshold).
 *   - Resolve: 80ms after selection settles (don't fetch docs
 *     for items the captain arrow-scrolled past).
 */

export interface CompletionState {
  open: boolean
  items: CompletionItem[]
  selectedIndex: number
  sourceId: string | null
  documentation: string | null
  resolving: boolean
  /** Last sent query id; received responses are matched against this. */
  latestQueryId: string | null
  /** Last sent resolve id; received responses are matched against this. */
  latestResolveId: string | null
}

export interface UseCompletionApi {
  state: Ref<CompletionState>
  selected: ComputedRef<CompletionItem | undefined>
  /**
   * Send a `completion/query`. Coalesces with pending debounce.
   * `sources` (when set) restricts which daemon-side sources fire
   * during detect — drives palette modes that want autocomplete
   * from a specific source only (cwd → `['path']`).
   */
  query: (text: string, cursor: number, opts?: { manual?: boolean; cwd?: string; instanceId?: string; sources?: CompletionSourceId[] }) => void
  /** Cancel the in-flight query (ripgrep specifically) and close the popover. */
  close: () => void
  /** Move selection within `items` — wraps at boundaries. */
  selectNext: () => void
  selectPrev: () => void
  /** Commit the active item; returns the `Replacement` to apply, or undefined when nothing selected. */
  commit: () => CompletionItem | undefined
}

const DEFAULT_QUERY_DEBOUNCE_MS = 30
const RESOLVE_DEBOUNCE_MS = 80

let singleton: UseCompletionApi | undefined
// Captain-tunable debounce override sourced from `[completion.ripgrep]
// debounce_ms` at boot. Auto-triggered queries (regular typing) honour
// this; manual queries (Tab / Ctrl+Space) skip the debounce so the
// captain's explicit ask fires immediately.
let autoDebounceMs = DEFAULT_QUERY_DEBOUNCE_MS

export function setCompletionDebounceMs(ms: number): void {
  autoDebounceMs = Math.max(0, Math.floor(ms))
}

/**
 * Boot-time fetch of the `[completion]` config block. Applies the
 * captain's `ripgrep.debounceMs` to the auto-trigger pipeline so
 * heavy ripgrep queries throttle without slowing the path / skills /
 * commands sources. Soft-fails (logs + continues) when no Tauri
 * host is bound — browser dev / vitest stays on the default debounce.
 */
export async function loadCompletionConfig(): Promise<void> {
  try {
    const cfg = await invoke(TauriCommand.GetCompletionConfig)

    if (typeof cfg?.ripgrep?.debounceMs === 'number') {
      setCompletionDebounceMs(cfg.ripgrep.debounceMs)
    }
  } catch(err) {
    log.warn('get_completion_config invoke failed; using default debounce', undefined, err)
  }
}

export function useCompletion(): UseCompletionApi {
  if (singleton) {
    return singleton
  }

  const state = ref<CompletionState>({
    open: false,
    items: [],
    selectedIndex: 0,
    sourceId: null,
    documentation: null,
    resolving: false,
    latestQueryId: null,
    latestResolveId: null
  })

  const selected = computed<CompletionItem | undefined>(() => state.value.items[state.value.selectedIndex])

  let queryDebounce: ReturnType<typeof setTimeout> | undefined
  let resolveDebounce: ReturnType<typeof setTimeout> | undefined

  function query(text: string, cursor: number, opts?: { manual?: boolean; cwd?: string; instanceId?: string; sources?: CompletionSourceId[] }): void {
    if (queryDebounce) {
      clearTimeout(queryDebounce)
    }

    // Drop the open popover immediately — the items are computed
    // against an older buffer state than what the captain just
    // typed. Without this, the list visually lingers through the
    // debounce window with stale rows highlighted. Re-opens once
    // the new response lands.
    if (state.value.open) {
      state.value.open = false
      state.value.items = []
      state.value.selectedIndex = 0
      state.value.documentation = null
    }
    // Manual fires immediately; auto runs through the captain's
    // configured debounce so ripgrep-bearing queries throttle without
    // gating cheap path / skills sources too aggressively.
    const debounce = opts?.manual ? 0 : autoDebounceMs

    queryDebounce = setTimeout(() => {
      void runQuery(text, cursor, opts)
    }, debounce)
  }

  async function runQuery(text: string, cursor: number, opts?: { manual?: boolean; cwd?: string; instanceId?: string; sources?: CompletionSourceId[] }): Promise<void> {
    let response: CompletionQueryResponse

    try {
      response = await invoke(TauriCommand.CompletionQuery, {
        text,
        cursor,
        manual: opts?.manual ?? false,
        cwd: opts?.cwd,
        instanceId: opts?.instanceId,
        sources: opts?.sources
      })
    } catch(err) {
      log.warn('completion/query failed', { err: String(err) })

      return
    }

    // Watermark — daemon assigns a fresh requestId per query; UI tracks
    // the latest. Older responses arriving after a newer query lands
    // are dropped here. (Daemon ranks per request; we never re-rank
    // client-side.)
    state.value.latestQueryId = response.requestId

    if (response.sourceId === null || response.items.length === 0) {
      state.value.open = false
      state.value.items = []
      state.value.selectedIndex = 0
      state.value.sourceId = null
      state.value.documentation = null

      return
    }

    state.value.open = true
    state.value.items = response.items
    state.value.selectedIndex = 0
    state.value.sourceId = response.sourceId
    state.value.documentation = null
    scheduleResolve()
  }

  function close(): void {
    if (queryDebounce) {
      clearTimeout(queryDebounce)
      queryDebounce = undefined
    }

    if (resolveDebounce) {
      clearTimeout(resolveDebounce)
      resolveDebounce = undefined
    }

    if (state.value.latestQueryId) {
      const requestId = state.value.latestQueryId

      void invoke(TauriCommand.CompletionCancel, { requestId }).catch((err: unknown) => {
        log.trace('completion/cancel rejected', { err: String(err) })
      })
    }
    state.value.open = false
    state.value.items = []
    state.value.selectedIndex = 0
    state.value.sourceId = null
    state.value.documentation = null
    state.value.resolving = false
    state.value.latestQueryId = null
    state.value.latestResolveId = null
  }

  function selectNext(): void {
    if (state.value.items.length === 0) {
      return
    }
    state.value.selectedIndex = (state.value.selectedIndex + 1) % state.value.items.length
    state.value.documentation = null
    scheduleResolve()
  }

  function selectPrev(): void {
    if (state.value.items.length === 0) {
      return
    }
    state.value.selectedIndex = (state.value.selectedIndex - 1 + state.value.items.length) % state.value.items.length
    state.value.documentation = null
    scheduleResolve()
  }

  function commit(): CompletionItem | undefined {
    const item = selected.value

    if (item) {
      close()
    }

    return item
  }

  function scheduleResolve(): void {
    if (resolveDebounce) {
      clearTimeout(resolveDebounce)
    }
    const item = selected.value

    if (!item || !item.resolveId || !state.value.sourceId) {
      return
    }
    const sourceId = state.value.sourceId
    const resolveId = item.resolveId

    state.value.latestResolveId = resolveId
    state.value.resolving = true
    resolveDebounce = setTimeout(() => {
      void runResolve(sourceId, resolveId)
    }, RESOLVE_DEBOUNCE_MS)
  }

  async function runResolve(sourceId: string, resolveId: string): Promise<void> {
    let response: CompletionResolveResponse

    try {
      response = await invoke(TauriCommand.CompletionResolve, {
        resolveId,
        sourceId: sourceId as 'skills' | 'path' | 'ripgrep' | 'commands'
      })
    } catch(err) {
      log.warn('completion/resolve failed', { err: String(err) })
      state.value.resolving = false

      return
    }

    // Drop stale resolves — selection may have advanced past the
    // item we requested docs for.
    if (state.value.latestResolveId !== resolveId) {
      return
    }
    state.value.documentation = response.documentation ?? null
    state.value.resolving = false
  }

  singleton = {
    state,
    selected,
    query,
    close,
    selectNext,
    selectPrev,
    commit
  }

  return singleton
}

/**
 * Test-only reset — drops the singleton so a fresh instance can be
 * constructed in the next test.
 */
export function __resetUseCompletionForTests(): void {
  singleton = undefined
}
