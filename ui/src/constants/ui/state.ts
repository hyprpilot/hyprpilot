/**
 * Cross-cutting UI state enums + their tone/CSS-suffix mappers.
 * `Phase` drives the header chrome (border + profile pill bg + chat
 * indicators); `ToolState` colours tool chips. Helpers co-locate so
 * the enum and its standard rendering stay one file away.
 */

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
  Running = 'running',
  Done = 'done',
  Failed = 'failed',
  Awaiting = 'awaiting'
}

export function toolStateTone(state: ToolState): string {
  switch (state) {
    case ToolState.Running:
      return 'var(--theme-state-stream)'
    case ToolState.Failed:
      return 'var(--theme-status-err)'
    case ToolState.Awaiting:
      return 'var(--theme-state-awaiting)'
    case ToolState.Done:
      return 'var(--theme-status-ok)'
    default:
      return 'var(--theme-fg-dim)'
  }
}
