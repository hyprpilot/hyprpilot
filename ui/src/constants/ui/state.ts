/**
 * Cross-cutting UI state enums + their tone/CSS-suffix mappers.
 * `Phase` drives the header chrome (border + profile pill bg + chat
 * indicators); `ToolState` colours tool chips.
 *
 * `ToolState` is the UI's tone classification — derived from the wire
 * `ToolCallState` (Pending / Running / Completed / Failed) plus any
 * UI-only signal (e.g. `Awaiting` when a permission prompt is open).
 * The wire raw state lives at `@constants/wire/transcript::ToolCallState`.
 */

import { ToolCallState } from '@constants/wire/transcript'

export enum Phase {
  Idle = 'idle',
  Streaming = 'streaming',
  Pending = 'pending',
  Awaiting = 'awaiting',
  Working = 'working'
}

/// Rust exposes the streaming state as `stream` (see
/// `config.state.stream`); every other phase value already matches
/// its CSS suffix 1:1.
export function phaseToCssSuffix(p: Phase): string {
  return p === Phase.Streaming ? 'stream' : p.toLowerCase()
}

export enum ToolState {
  Pending = 'pending',
  Running = 'running',
  Done = 'done',
  Failed = 'failed',
  Awaiting = 'awaiting',
  Cancelled = 'cancelled'
}

/// Wire `ToolCallState` → UI tone `ToolState`. `Completed` maps to
/// `Done`; everything else passes through. `Awaiting` is a UI-only
/// state set externally when a permission prompt is open against the
/// call.
export function toolStateFromWire(state: ToolCallState | undefined): ToolState {
  switch (state) {
    case ToolCallState.Pending:
      return ToolState.Pending

    case ToolCallState.Running:
      return ToolState.Running

    case ToolCallState.Completed:
      return ToolState.Done

    case ToolCallState.Failed:
      return ToolState.Failed

    default:
      return ToolState.Running
  }
}

export function toolStateTone(state: ToolState): string {
  switch (state) {
    case ToolState.Running:
      return 'var(--theme-state-stream)'

    case ToolState.Pending:
      return 'var(--theme-state-pending)'

    case ToolState.Awaiting:
      return 'var(--theme-state-awaiting)'

    case ToolState.Failed:
      return 'var(--theme-status-err)'

    case ToolState.Cancelled:
      return 'var(--theme-fg-dim)'

    case ToolState.Done:
      return 'var(--theme-status-ok)'

    default:
      return 'var(--theme-fg-dim)'
  }
}
