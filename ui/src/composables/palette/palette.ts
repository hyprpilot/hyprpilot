/**
 * Recursive command-palette stack — the overlay owns a singleton stack of
 * palette specs, each describing one level of navigation (root picker,
 * sub-picker, and so on). The top of the stack is what `CommandPalette.vue`
 * renders; `open()` pushes a new spec, `close()` pops one level, and
 * `closeAll()` clears the stack. State sits at module scope so any
 * consumer (the overlay, a sub-palette's `onCommit`, a keybinding) shares
 * the same stack.
 */

import { ref, type Component, type Ref } from 'vue'

export enum PaletteMode {
  Select = 'select',
  MultiSelect = 'multi-select',
  /**
   * Input + autocomplete shape — the captain types a value into the
   * search input, autocomplete suggestions render below as
   * picker rows, and Enter commits either the highlighted suggestion
   * or the typed value verbatim. Empty query hides the suggestion
   * list entirely (no static entries shown). Used by the cwd
   * palette and any future "type a path / value" surface.
   */
  Input = 'input'
}

export interface PaletteEntry {
  id: string
  name: string
  description?: string
  kind?: string
  /**
   * When `true`, the palette paints a primary-color left-border on
   * the row regardless of fuzzy-filter cursor position. Drives the
   * "this is your persisted choice" marker (active instance, active
   * profile, current cwd, …) so the captain reads it at a glance
   * even while arrow-navigating other rows.
   */
  active?: boolean
}

/**
 * Optional right-pane preview. When set, `CommandPalette.vue` renders
 * a wide shell with the preview component bound to the currently
 * highlighted entry. The component receives `{ entry, ...props }` as
 * props — `entry` is `undefined` when the list is empty / unfiltered
 * out. `props` is an optional bag of extra static props (e.g. the
 * full item collection so the preview can look up structured fields
 * by id without re-fetching).
 */
export interface PalettePreview {
  component: Component
  props?: Record<string, unknown>
}

export interface PaletteSpec {
  mode: PaletteMode
  title?: string
  entries: PaletteEntry[]
  preseedActive?: PaletteEntry[]
  preview?: PalettePreview
  /**
   * Placeholder text rendered inside the palette's search input.
   * Useful for `Input` mode where the input IS the primary surface
   * (other modes leave it blank). Static leaves omit.
   */
  placeholder?: string
  /**
   * `query` is the live search-input value at commit time. Most
   * leaves ignore it; the cwd leaf reads it as the manual-path input
   * when the `manual` sentinel row is the highlighted pick.
   */
  onCommit: (picks: PaletteEntry[], query?: string) => void | Promise<void>
  /**
   * Live-query hook — fires every time the search input changes.
   * Receives the new query plus an `update(entries)` callback the
   * leaf calls to swap its own entries reactively (e.g. cwd palette
   * piping a typed path through directory autocompletion). Most
   * leaves don't set this and the static `entries` array stays
   * authoritative.
   */
  onQueryChange?: (query: string, update: (entries: PaletteEntry[]) => void) => void
  /**
   * Ctrl+D handler. Receives the highlighted entry plus an
   * `update(entries)` callback the leaf calls to swap the spec's
   * entries reactively after a delete (e.g. instances palette
   * dropping a shut-down row). The callback is mandatory for any
   * leaf that mutates state — calling `spec.entries = next` on the
   * captured local literal bypasses Vue's reactive proxy and the
   * filter watcher never re-fires, leaving the palette showing stale
   * rows. Same pattern as `onQueryChange`.
   */
  onDelete?: (entry: PaletteEntry, update: (entries: PaletteEntry[]) => void) => void | Promise<void>
  /**
   * `true` while the spec's entries are still being fetched. The
   * palette swaps the empty-state row for an inline <Loading>
   * spinner with `loadingStatus` as the description so the user
   * sees what's happening (instead of a misleading "no matches").
   * Sessions / models / palette leaves with async population
   * should set this; static leaves omit.
   */
  loading?: boolean
  /** Status text rendered next to the inline spinner. */
  status?: string
  /**
   * Skip the daemon-side `completion/rank` call — the spec's entries
   * are already server-pre-filtered against the typed query (cwd
   * path completion, future ripgrep/grep-like leaves) and ranking
   * the basenames against a full path query would over-prune.
   * Static leaves leave this unset.
   */
  filtered?: boolean
}

const stack = ref<PaletteSpec[]>([])
let lastFocused: HTMLElement | undefined

// Snapshot + eagerly clear before focusing: a `focus()` that throws or
// triggers listeners that re-enter `close()` must not leave a stale ref
// behind. Skip disconnected / disabled nodes — the element the user
// clicked may have been unmounted while the palette was up.
function restoreFocus(): void {
  const el = lastFocused

  lastFocused = undefined

  if (!el || !el.isConnected || (el as HTMLButtonElement).disabled) {
    return
  }

  try {
    el.focus()
  } catch {
    // ignore — element may have been detached mid-call
  }
}

export function usePalette(): {
  stack: Ref<PaletteSpec[]>
  open: (spec: PaletteSpec) => void
  close: () => void
  closeAll: () => void
} {
  return {
    stack,
    open(spec: PaletteSpec): void {
      if (stack.value.length === 0) {
        const active = document.activeElement

        if (active instanceof HTMLElement) {
          lastFocused = active
        } else {
          lastFocused = undefined
        }
      }
      stack.value.push(spec)
    },
    close(): void {
      if (stack.value.length === 0) {
        return
      }
      stack.value.pop()

      if (stack.value.length === 0) {
        restoreFocus()
      }
    },
    closeAll(): void {
      if (stack.value.length === 0) {
        return
      }
      stack.value.length = 0
      restoreFocus()
    }
  }
}

/** Test-only: clear the stack without touching focus state. */
export function __resetPaletteStackForTests(): void {
  stack.value.length = 0
  lastFocused = undefined
}
