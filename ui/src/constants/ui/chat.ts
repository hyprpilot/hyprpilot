/**
 * Chat-specific UI enums + their accompanying tiny mappers
 * (icon-per-tool-kind, FontAwesome icon discriminator).
 */

import type { IconDefinition } from '@fortawesome/fontawesome-svg-core'
import {
  faBrain,
  faCube,
  faFileLines,
  faMagnifyingGlass,
  faPen,
  faPenToSquare,
  faPlug,
  faTerminal,
  faUserGear
} from '@fortawesome/free-solid-svg-icons'

export enum StreamKind {
  Thinking = 'thinking',
  Planning = 'planning'
}

export enum PlanStatus {
  Pending = 'pending',
  InProgress = 'in_progress',
  Completed = 'completed'
}

/**
 * Closed set mirroring `[ui.theme.kind]` in `defaults.toml`. Drives
 * both the per-tool-family tint (via `var(--theme-kind-<key>)`) and
 * the big-row dispatch in chat tool chips.
 */
export enum ToolKind {
  Read = 'read',
  Write = 'write',
  Bash = 'bash',
  Search = 'search',
  Agent = 'agent',
  Think = 'think',
  Terminal = 'terminal',
  Acp = 'acp'
}

/** Narrow a `KeyLabel` to its FontAwesome `IconDefinition` branch. */
export function isFaIcon(k: unknown): k is IconDefinition {
  return typeof k === 'object' && k !== null && 'iconName' in k && 'prefix' in k
}

/**
 * Leading glyph per tool-kind dispatch tag on chips/rows. Keys cover
 * every `ToolKind` variant plus a couple of legacy / alias strings
 * (`edit`, `grep`) so callers that still hand a raw string through
 * get a useful icon. Unknown or missing kinds fall back to the
 * generic `cube` glyph.
 */
const TOOL_KIND_ICONS: Record<string, IconDefinition> = {
  bash: faTerminal,
  write: faPenToSquare,
  read: faFileLines,
  edit: faPen,
  search: faMagnifyingGlass,
  grep: faMagnifyingGlass,
  terminal: faTerminal,
  agent: faUserGear,
  think: faBrain,
  acp: faPlug
}

export function iconForToolKind(kind: ToolKind | string | undefined): IconDefinition {
  return TOOL_KIND_ICONS[(kind ?? '').toLowerCase()] ?? faCube
}
