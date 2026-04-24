/**
 * Recursive command-palette stack — the overlay owns a singleton stack of
 * palette specs, each describing one level of navigation (root picker,
 * sub-picker, and so on). The top of the stack is what `CommandPalette.vue`
 * renders; `open()` pushes a new spec, `close()` pops one level, and
 * `closeAll()` clears the stack. State sits at module scope so any
 * consumer (the overlay, a sub-palette's `onCommit`, a keybinding) shares
 * the same stack.
 */

import { ref, type Ref } from 'vue'

export enum PaletteMode {
  Select = 'select',
  MultiSelect = 'multi-select'
}

export interface PaletteEntry {
  id: string
  name: string
  description?: string
  kind?: string
}

export interface PaletteSpec {
  mode: PaletteMode
  title?: string
  entries: PaletteEntry[]
  preseedActive?: PaletteEntry[]
  onCommit(picks: PaletteEntry[]): void | Promise<void>
  onDelete?(entry: PaletteEntry): void | Promise<void>
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
  open(spec: PaletteSpec): void
  close(): void
  closeAll(): void
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
