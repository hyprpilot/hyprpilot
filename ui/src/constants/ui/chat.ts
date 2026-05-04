/**
 * Chat-specific UI enums.
 */

import type { IconDefinition } from '@fortawesome/fontawesome-svg-core'

export enum StreamKind {
  Thinking = 'thinking',
  Planning = 'planning'
}

export enum PlanStatus {
  Pending = 'pending',
  InProgress = 'in_progress',
  Completed = 'completed'
}

/** Narrow a `KeyLabel` to its FontAwesome `IconDefinition` branch. */
export function isFaIcon(k: unknown): k is IconDefinition {
  return typeof k === 'object' && k !== null && 'iconName' in k && 'prefix' in k
}
