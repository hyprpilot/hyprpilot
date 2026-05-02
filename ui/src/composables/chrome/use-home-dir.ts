import { ref, type Ref } from 'vue'

import { invoke, TauriCommand } from '@ipc'

/// Module-level singleton. The webview can't read `$HOME` itself, so
/// we ferry it in once via the Tauri command and share the resolved
/// value across consumers. `undefined` until `loadHomeDir()` resolves
/// (or forever, if Tauri host is missing — the consumer treats that
/// as "no `~` collapse").
const homeDir = ref<string>()

/// Same shape for the daemon's working directory — drives the idle
/// banner so the captain sees where new instances will land before
/// they spawn. Reflects whatever `--cwd <DIR>` was passed (or the
/// spawning shell's cwd when omitted).
const daemonCwd = ref<string>()

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

export async function loadDaemonCwd(): Promise<void> {
  try {
    daemonCwd.value = await invoke(TauriCommand.GetDaemonCwd)
  } catch {
    daemonCwd.value = undefined
  }
}

export function useHomeDir(): { homeDir: Ref<string | undefined> } {
  return { homeDir }
}

export function useDaemonCwd(): { daemonCwd: Ref<string | undefined> } {
  return { daemonCwd }
}

/** Test-only helper. */
export function __resetHomeDirForTests(): void {
  homeDir.value = undefined
  daemonCwd.value = undefined
}
