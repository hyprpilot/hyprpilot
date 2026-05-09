/**
 * Snapshot of the daemon's `[completion]` config block. UI uses
 * `ripgrep.debounceMs` to throttle auto-trigger queries since
 * ripgrep walks the cwd's file tree per call.
 */
export interface CompletionConfigSnapshot {
  ripgrep: {
    auto: boolean
    debounceMs: number
    minPrefix: number
  }
}
