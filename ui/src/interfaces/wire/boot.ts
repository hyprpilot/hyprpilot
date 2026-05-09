import type { CompletionConfigSnapshot } from './completion-config'
import type { KeymapsConfig } from './keymap'
import type { AgentSummary, InstanceListEntry, ProfileSummary } from './session'
import type { Theme } from './theme'
import type { WindowState } from './window'

/**
 * Aggregated boot payload. One `invoke('boot_snapshot')` returns
 * everything the loading screen needs before it can drop. Replaces
 * sequential `await invoke(...)` round-trips — load-bearing on the
 * remote bridge where each round-trip rides the same WS.
 *
 * Per-instance snapshot data (chat / terminals) stays on its own
 * RPCs; brim-sync calls those after boot for whichever instance is
 * focused.
 */
export interface BootSnapshot {
  theme: Theme
  keymaps: KeymapsConfig
  windowState: WindowState
  homeDir: string
  daemonCwd: string
  completionConfig: CompletionConfigSnapshot
  agents: { agents: AgentSummary[] }
  profiles: { profiles: ProfileSummary[] }
  instances: { instances: InstanceListEntry[]; focusedId?: string }
}
