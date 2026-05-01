/**
 * Keyboard-hint primitive types. Shared by `KbdHint` + every keyboard
 * legend / footer hint surface.
 */

import type { IconDefinition } from '@fortawesome/fontawesome-svg-core'

/**
 * A single keycap in a `KbdHint`. Strings render as plain text (Ctrl,
 * Esc, Ctrl+K); `IconDefinition` (a directly-imported FontAwesome
 * icon) renders as a glyph inside the `<kbd>`. No string-array
 * `['fas', 'foo']` form — direct imports per the no-`library.add`
 * rule (CLAUDE.md / AGENTS.md).
 */
export type KeyLabel = string | IconDefinition

/** A set of keyboard-hint chips, e.g. `↑ ↓ move`. */
export interface KbdHintSpec {
  keys: KeyLabel[]
  label: string
}
