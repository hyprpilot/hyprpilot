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
  query: (text: string, cursor: number, opts?: CompletionQueryOptions) => void
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

type CompletionInitialSelection = 'none' | 'first' | 'last'

interface CompletionQueryOptions {
  manual?: boolean
  cwd?: string
  instanceId?: string
  sources?: CompletionSourceId[]
  initialSelection?: CompletionInitialSelection
}

let singleton: UseCompletionApi | undefined
// Captain-tunable debounce override sourced from `[completion.ripgrep]
// debounce_ms` at boot. Auto-triggered queries (regular typing) honour
// this; manual queries (Ctrl+Space) skip the debounce so the
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

    applyCompletionConfigFromObject(cfg)
  } catch(err) {
    log.warn('get_completion_config invoke failed; using default debounce', undefined, err)
  }
}

/** Apply a config snapshot already in hand (boot snapshot). */
export function applyCompletionConfigFromObject(cfg: { ripgrep?: { debounceMs?: number } } | undefined): void {
  if (typeof cfg?.ripgrep?.debounceMs === 'number') {
    setCompletionDebounceMs(cfg.ripgrep.debounceMs)
  }
}

export function useCompletion(): UseCompletionApi {
  if (singleton) {
    return singleton
  }

  // `selectedIndex: -1` is the "nothing highlighted yet" sentinel —
  // not just an empty list. The popover opens with rows visible but
  // no row tinted, so Enter is unambiguous: it submits the buffer.
  // Ctrl+Space is the explicit "I want a completion" verb; Tab only
  // walks rows after the popover is already visible.
  const state = ref<CompletionState>({
    open: false,
    items: [],
    selectedIndex: -1,
    sourceId: null,
    documentation: null,
    resolving: false,
    latestQueryId: null,
    latestResolveId: null
  })

  const selected = computed<CompletionItem | undefined>(() => state.value.items[state.value.selectedIndex])

  let queryDebounce: ReturnType<typeof setTimeout> | undefined
  let resolveDebounce: ReturnType<typeof setTimeout> | undefined
  /// Generation counter — every `query()` issue and every `close()`
  /// bumps it. In-flight `runQuery` captures the value at issue time
  /// and compares on response; a mismatch means the call was
  /// superseded or the popover was closed between `invoke()` issue
  /// and resolution, so the stale response must NOT reopen the
  /// popover. Without this, hitting Enter with a query mid-flight
  /// re-shows the previous completion items after submit clears
  /// the buffer — the response handler unconditionally writes
  /// `state.open = true` and the cancel RPC is best-effort.
  let queryGeneration = 0

  function query(text: string, cursor: number, opts?: CompletionQueryOptions): void {
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
      state.value.selectedIndex = -1
      state.value.documentation = null
    }
    // Manual fires immediately; auto runs through the captain's
    // configured debounce so ripgrep-bearing queries throttle without
    // gating cheap path / skills sources too aggressively.
    const debounce = opts?.manual ? 0 : autoDebounceMs

    queryGeneration += 1
    const myGeneration = queryGeneration

    queryDebounce = setTimeout(() => {
      void runQuery(text, cursor, myGeneration, opts)
    }, debounce)
  }

  async function runQuery(text: string, cursor: number, generation: number, opts?: CompletionQueryOptions): Promise<void> {
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

    // Drop responses superseded by a newer query OR by a `close()`
    // call between issue and resolution (e.g. Enter-to-send fired
    // while ripgrep was mid-walk). Without this, the response handler
    // would write `state.open = true` and re-show stale items.
    if (generation !== queryGeneration) {
      log.trace('completion: dropping superseded response', {
        responseGeneration: generation,
        currentGeneration: queryGeneration
      })

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
      state.value.selectedIndex = -1
      state.value.sourceId = null
      state.value.documentation = null

      return
    }

    state.value.open = true
    state.value.items = response.items
    const initialSelection = opts?.initialSelection ?? 'none'

    state.value.selectedIndex = initialSelection === 'first' ? 0 : initialSelection === 'last' ? response.items.length - 1 : -1
    state.value.sourceId = response.sourceId ?? null
    state.value.documentation = null

    if (state.value.selectedIndex >= 0) {
      scheduleResolve()
    }
  }

  function close(): void {
    // Bump generation so any in-flight `runQuery` whose `invoke()`
    // already left the wire drops its response on arrival instead
    // of reopening the popover with stale items.
    queryGeneration += 1

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
    state.value.selectedIndex = -1
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
    const cur = state.value.selectedIndex

    // From the "nothing highlighted" sentinel (-1), land on the first
    // row — Tab / ArrowDown is the verb that turns the popover into
    // an actual completion pick.
    state.value.selectedIndex = cur < 0 ? 0 : (cur + 1) % state.value.items.length
    state.value.documentation = null
    scheduleResolve()
  }

  function selectPrev(): void {
    if (state.value.items.length === 0) {
      return
    }
    const cur = state.value.selectedIndex

    // ArrowUp from the sentinel wraps to the last row (mirrors the
    // forward sentinel-handling above so both directions feel symmetric).
    state.value.selectedIndex = cur < 0 ? state.value.items.length - 1 : (cur - 1 + state.value.items.length) % state.value.items.length
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
        sourceId: sourceId as CompletionSourceId
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
