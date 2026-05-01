/**
 * Tool-formatter contract — every per-tool formatter file under
 * `lib/tools/formatters/<name>.ts` implements `ToolFormatter`. The
 * registry (`lib/tools/registry.ts`) derives the lookup tables from
 * a list of definitions.
 */

import type { ToolKind } from '@constants/ui/chat'
import type { ToolState } from '@constants/ui/state'

import type { ToolChipItem } from './chat'

export type Args = Record<string, unknown>

export interface FormatterContext {
  /** Normalised canonical tool name (lowercase + dashes-to-underscores). */
  name: string
  /** Raw tool name as emitted by the agent, before normalisation. */
  rawName: string
  /**
   * ACP wire kind from `tool_call.kind` — closed enum
   * (`read | edit | delete | move | search | execute | think | fetch
   * | switch_mode | other`). Empty string when the agent didn't
   * supply one. Drives the fallback formatter's label + ToolKind so
   * unknown tools land on the right glyph instead of the agent's
   * prose title.
   */
  kind: string
  /** Argument map with every key normalised via `normaliseKey`. */
  args: Args
  state: ToolState
}

/**
 * One tool's formatter. Each registered tool ships its own file under
 * `lib/tools/formatters/<name>.ts` implementing this shape; the
 * registry derives the lookup map from the list of definitions.
 *
 * Authoring rules:
 *  - `canonical` is the snake_case key the registry stores under.
 *  - `aliases` covers any agent-side spelling that should resolve to
 *    this formatter without a separate alias entry.
 *  - `label` is the chip's text identifier (aria-label, screen
 *    reader, plain-text exports). OPTIONAL — the registry default
 *    is the canonical key title-cased.
 *  - `kind` selects the chip tone + FontAwesome icon via
 *    `iconForToolKind`.
 *  - `format(ctx)` produces the chip. Always honor `ctx.state` so
 *    in-flight / awaiting / done tints render correctly.
 */
export interface ToolFormatter {
  readonly canonical: string
  readonly aliases?: readonly string[]
  /** Optional override; defaults to `titleCaseFromCanonical(canonical)`. */
  readonly label?: string
  readonly kind: ToolKind
  format(ctx: FormatterContext): ToolChipItem
}

/**
 * Resolved registry — the lookup tables `formatToolCall` walks at
 * runtime. Built once from a list of `ToolFormatter` definitions in
 * `registry.ts`.
 */
export interface ToolFormatterRegistry {
  /** Canonical name → short verb word. Surface for `aria-label` etc. */
  shortHeaders: Record<string, string>
  /** Wire-name → canonical formatter key. */
  aliases: Record<string, string>
  /** Canonical name → formatter. */
  formatters: Record<string, ToolFormatter>
  /** Last-resort formatter — handles MCP-style names + wire-kind fallback. */
  fallback: ToolFormatter
}
