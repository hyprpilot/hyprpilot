import { ref, type Ref } from 'vue'

import { invoke, TauriCommand } from '@ipc'

/// Module-level singleton. The webview can't read `$HOME` itself, so
/// we ferry it in once via the Tauri command and share the resolved
/// value across consumers. `undefined` until `loadHomeDir()` resolves
/// (or forever, if Tauri host is missing — the consumer treats that
/// as "no `~` collapse").
const homeDir = ref<string>()

/**
 * One-shot fetch of the user's `$HOME` from the Rust side. Idempotent
 * — repeated calls re-resolve the same value. Soft-fails when the
 * Tauri host isn't bound (plain vite dev / vitest jsdom): the ref
 * stays `undefined` and `truncateCwd` skips the home-prefix collapse.
 */
export async function loadHomeDir(): Promise<void> {
  try {
    homeDir.value = await invoke(TauriCommand.GetHomeDir)
  } catch {
    homeDir.value = undefined
  }
}

export function useHomeDir(): { homeDir: Ref<string | undefined> } {
  return { homeDir }
}

/** Test-only helper. */
export function __resetHomeDirForTests(): void {
  homeDir.value = undefined
}
