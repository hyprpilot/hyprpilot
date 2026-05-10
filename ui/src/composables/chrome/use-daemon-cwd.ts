import { ref, type Ref } from 'vue'

/**
 * Daemon working directory — drives the idle banner so the captain
 * sees where new instances will land before they spawn one.
 *
 * Wire shape: pre-formatted display string (`~/development/hyprpilot`),
 * not the absolute form. The daemon collapses `$HOME` server-side via
 * `tools::path::display_cwd`, so every frontend (Vue overlay today,
 * Neovim plugin tomorrow, `ctl --json`) renders the same shape
 * verbatim. The UI never reaches for `$HOME` to do its own collapse.
 *
 * Seeded by the boot snapshot via `setDaemonCwd`. `undefined` until
 * boot resolves (or forever, if the daemon is unreachable — the idle
 * banner falls back to a generic label).
 */
const daemonCwd = ref<string>()

/** Boot-snapshot setter — apply the value without IPC. */
export function setDaemonCwd(value: string | undefined): void {
  daemonCwd.value = value
}

export function useDaemonCwd(): { daemonCwd: Ref<string | undefined> } {
  return { daemonCwd }
}

/** Test-only helper. */
export function __resetDaemonCwdForTests(): void {
  daemonCwd.value = undefined
}
