/**
 * `[keymaps]` config tree mirror. Every leaf is a typed `Binding`
 * (`{ modifiers, key }`) — `key` is a lowercase string matching
 * `KeyboardEvent.key.toLowerCase()` (`arrowup` for the named key,
 * `a` / `?` for single-char glyphs). Nested subgroups
 * (`palette.instances`) are their own collision scope;
 * bindings only clash within the same parent struct. See
 * `src-tauri/src/config/keymaps.rs` for the Rust-side source of truth.
 */
import type { Modifier } from '@constants/wire/keymap'

/**
 * A single keybinding: modifier set + key token. `key` is the lowercase
 * value Rust serialises — `arrowup` / `enter` / `tab` for named keys,
 * a single glyph (`a`, `?`, `k`) for printable characters. Matched
 * against `KeyboardEvent.key.toLowerCase()` directly; `space` is the
 * one bridge (DOM emits literal `' '`). Modifier order is canonicalised
 * Rust-side at deserialize so equality is stable.
 */
export interface Binding {
  modifiers: Modifier[]
  key: string
}

export interface ChatKeymaps {
  submit: Binding
  newline: Binding
  cancel_turn: Binding
}

export interface ApprovalsKeymaps {
  allow: Binding
  deny: Binding
}

export interface ComposerKeymaps {
  paste: Binding
  tab_completion: Binding
  shift_tab: Binding
  completion: Binding
  history_up: Binding
  history_down: Binding
}

export interface InstancesSubPaletteKeymaps {
  focus: Binding
}

export interface PaletteKeymaps {
  open: Binding
  close: Binding
  instances: InstancesSubPaletteKeymaps
}

export type TranscriptKeymaps = Record<string, never>

export interface WindowKeymaps {
  toggle: Binding
}

export interface QueueKeymaps {
  send: Binding
  drop: Binding
}

export interface KeymapsConfig {
  chat: ChatKeymaps
  approvals: ApprovalsKeymaps
  composer: ComposerKeymaps
  palette: PaletteKeymaps
  transcript: TranscriptKeymaps
  window: WindowKeymaps
  queue: QueueKeymaps
}
