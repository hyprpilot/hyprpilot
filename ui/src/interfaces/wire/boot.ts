import type { CompletionConfigSnapshot } from './completion-config'
import type { ChatSnapshot } from './instance-snapshot'
import type { KeymapsConfig } from './keymap'
import type { NotificationsSnapshot } from './notifications'
import type { QueueItem } from './queue'
import type { AgentSummary, InstanceListEntry, ProfileSummary } from './session'
import type { Theme } from './theme'
import type { WindowState } from './window'

/**
 * Aggregated boot payload. One `invoke('boot_snapshot')` returns
 * everything the loading screen needs before it can drop. Replaces
 * sequential `await invoke(...)` round-trips — load-bearing on the
 * remote bridge where each round-trip rides the same WS.
 *
 * Per-instance terminal snapshots stay on their own RPC; the chat
 * first page + queue snapshots ride inline (one head window per live
 * instance) so the captain navigating into any instance sees full
 * history immediately — no per-focus prefetch race, no "I only see
 * the latest message" hydration gap when the daemon has no
 * `focusedId` pointer.
 */
export interface BootSnapshot {
  theme: Theme
  keymaps: KeymapsConfig
  windowState: WindowState
  /// Daemon working directory in display form (`$HOME` collapsed
  /// to `~`). The daemon owns formatting; the UI renders verbatim.
  daemonCwd: string
  completionConfig: CompletionConfigSnapshot
  agents: { agents: AgentSummary[] }
  profiles: { profiles: ProfileSummary[] }
  /// Captain's currently-selected default profile id. Seeded from
  /// `[profile] default`; mutated at runtime via `profile_set`. The
  /// header pill / palette active marker drive off this value.
  selectedProfileId?: string
  instances: { instances: InstanceListEntry[]; focusedId?: string }
  /// Per-instance queue snapshots keyed by instance id. Empty queues
  /// are included (as `[]`) so the consumer treats absence as "no
  /// instance" rather than "queue unknown".
  queues: Record<string, QueueItem[]>
  /// Per-instance first chat-page snapshots keyed by instance id.
  /// Frontends seed their TanStack cache so the captain navigating
  /// into ANY live instance gets full history immediately. Empty
  /// `{ items: [], hasMore: false }` for instances whose mirror has
  /// no transcript yet.
  chats: Record<string, ChatSnapshot>
  /// Daemon-side "needs attention" snapshot. Empty `items: []` when
  /// nothing's flagged. Frontends seed the header pill / palette
  /// state directly so a remote captain authenticating mid-session
  /// sees the pill immediately if anything was already pending.
  notifications: NotificationsSnapshot
}
