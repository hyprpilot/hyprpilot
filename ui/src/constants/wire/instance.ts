/**
 * Per-instance lifecycle phase. Mirrors the Rust
 * `adapters::InstanceState` enum — emitted on every spawn, ready,
 * end, and error transition through the `acp:instance-state` event.
 */
export enum InstanceState {
  Starting = 'starting',
  Running = 'running',
  Ended = 'ended',
  Error = 'error'
}
