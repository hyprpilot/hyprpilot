/**
 * Wire-contract registry for every Tauri `invoke` command and `listen`
 * event the UI consumes. Mirrors the Rust side: `invoke_handler![...]`
 * in `src-tauri/src/daemon/mod.rs` and the `app.emit(...)` adapter
 * event emitters. Raw string literals at call sites are banned —
 * typos would only surface at runtime. The `*Result` / `*Payload`
 * interfaces below pick the response / event type off the command or
 * event name so `invoke` / `listen` infer it automatically.
 */

import type { BootSnapshot } from '@interfaces/wire/boot'
import type {
  CandidateItem,
  CompletionCancelArgs,
  CompletionCancelResponse,
  CompletionQueryArgs,
  CompletionQueryResponse,
  CompletionResolveArgs,
  CompletionResolveResponse
} from '@interfaces/wire/completion'
import type { CompletionConfigSnapshot } from '@interfaces/wire/completion-config'
import type {
  ComposerDraftAppendEventPayload,
  ConfigOptionsUpdateEventPayload,
  CurrentModeUpdateEventPayload,
  InstanceMetaEventPayload,
  InstanceStateEventPayload,
  InstancesChangedEventPayload,
  InstancesFocusedEventPayload,
  InstanceRenamedEventPayload,
  PermissionRequestEventPayload,
  SessionInfoUpdateEventPayload,
  SystemPromptInjectedEventPayload,
  TerminalEventPayload,
  TranscriptEventPayload,
  TurnEndedEventPayload,
  TurnStartedEventPayload,
  UsageUpdateEventPayload
} from '@interfaces/wire/event'
import type {
  ChatSnapshot,
  InstanceSnapshotChatArgs,
  InstanceSnapshotMetaArgs,
  InstanceSnapshotTerminalsArgs,
  MetaSnapshot,
  TerminalsSnapshot
} from '@interfaces/wire/instance-snapshot'
import type { KeymapsConfig } from '@interfaces/wire/keymap'
import type { NotificationsChangedEventPayload, NotificationsClearArgs, NotificationsGetArgs, NotificationsGetResult, NotificationsSnapshot } from '@interfaces/wire/notifications'
import type { AcpPermissionResolvedPayload } from '@interfaces/wire/permission-resolved'
import type {
  AcpQueueChangedPayload,
  QueueClearArgs,
  QueueClearResult,
  QueueDispatchArgs,
  QueueDispatchResult,
  QueueEditArgs,
  QueueEditResult,
  QueueListArgs,
  QueueListResult,
  QueueMoveArgs,
  QueueMoveResult,
  QueueRemoveArgs,
  QueueRemoveResult
} from '@interfaces/wire/queue'
import type { RemotePairRequestEventPayload, RemotePairResolvedEventPayload, RemotePendingPair } from '@interfaces/wire/remote-pair'
import type {
  AgentSummary,
  CancelArgs,
  CancelResult,
  InstanceListEntry,
  InstanceMetaArgs,
  InstanceMetaSnapshot,
  InstanceRestartArgs,
  InstanceRestartResult,
  InstancesFocusArgs,
  InstancesRenameArgs,
  InstancesRenameResult,
  InstancesShutdownArgs,
  ListSessionsArgs,
  LoadSessionArgs,
  McpsListArgs,
  MCPListResult,
  ModelsSetArgs,
  ModesSetArgs,
  ConfigOptionSetArgs,
  EffortGetArgs,
  EffortGetResult,
  EffortSetArgs,
  EffortsListArgs,
  EffortsListResult,
  PermissionReplyArgs,
  ProfileSummary,
  SessionInfoResult,
  SessionsInfoArgs,
  SessionSummary,
  SubmitArgs,
  SubmitResult
} from '@interfaces/wire/session'
import type { Theme } from '@interfaces/wire/theme'
import type { WindowState } from '@interfaces/wire/window'

export enum TauriCommand {
  BootSnapshot = 'boot_snapshot',
  GetTheme = 'get_theme',
  GetKeymaps = 'get_keymaps',
  GetWindowState = 'get_window_state',
  WindowToggle = 'window_toggle',
  DaemonRpc = 'daemon_rpc',
  ReadFileForAttachment = 'read_file_for_attachment',
  SessionSubmit = 'session_submit',
  SessionCancel = 'session_cancel',
  AgentsList = 'agents_list',
  ProfilesList = 'profiles_list',
  ProfileGet = 'profile_get',
  ProfileSet = 'profile_set',
  SessionList = 'session_list',
  SessionLoad = 'session_load',
  SessionsInfo = 'sessions_info',
  PermissionReply = 'permission_reply',
  InstancesList = 'instances_list',
  InstancesFocus = 'instances_focus',
  InstancesShutdown = 'instances_shutdown',
  InstancesRename = 'instances_rename',
  InstanceRestart = 'instance_restart',
  ModelsSet = 'models_set',
  ModesSet = 'modes_set',
  ConfigOptionSet = 'config_option_set',
  EffortGet = 'effort_get',
  EffortSet = 'effort_set',
  EffortsList = 'efforts_list',
  InstanceMeta = 'instance_meta',
  InstanceSnapshotMeta = 'instance_snapshot_meta',
  InstanceSnapshotChat = 'instance_snapshot_chat',
  InstanceSnapshotTerminals = 'instance_snapshot_terminals',
  InstanceSnapshotQueue = 'instance_snapshot_queue',
  McpsList = 'mcps_list',
  QueueList = 'queue_list',
  QueueEdit = 'queue_edit',
  QueueRemove = 'queue_remove',
  QueueMove = 'queue_move',
  QueueClear = 'queue_clear',
  QueueDispatch = 'queue_dispatch',
  NotificationsList = 'notifications_list',
  NotificationsGet = 'notifications_get',
  NotificationsClear = 'notifications_clear',
  NotificationsClearAll = 'notifications_clear_all',
  ResolveSpawnCwd = 'resolve_spawn_cwd',
  CompletionQuery = 'completion_query',
  CompletionResolve = 'completion_resolve',
  CompletionCancel = 'completion_cancel',
  CompletionRank = 'completion_rank',
  GetCompletionConfig = 'get_completion_config',
  SkillsReload = 'skills_reload',
  RemoteConfirmPair = 'remote_confirm_pair',
  RemoteRejectPair = 'remote_reject_pair',
  RemotePendingPairs = 'remote_pending_pairs'
}

export enum TauriEvent {
  AcpTranscript = 'acp:transcript',
  AcpPermissionRequest = 'acp:permission-request',
  AcpPermissionResolved = 'acp:permission-resolved',
  AcpInstanceState = 'acp:instance-state',
  AcpTurnStarted = 'acp:turn-started',
  AcpTurnEnded = 'acp:turn-ended',
  AcpTerminal = 'acp:terminal',
  AcpInstancesChanged = 'acp:instances-changed',
  AcpInstancesFocused = 'acp:instances-focused',
  AcpInstanceRenamed = 'acp:instance-renamed',
  AcpSessionInfoUpdate = 'acp:session-info-update',
  AcpCurrentModeUpdate = 'acp:current-mode-update',
  AcpUsageUpdate = 'acp:usage-update',
  AcpConfigOptionsUpdate = 'acp:config-options-update',
  AcpInstanceMeta = 'acp:instance-meta',
  AcpSystemPromptInjected = 'acp:system-prompt-injected',
  AcpQueueChanged = 'acp:queue-changed',
  AcpNotificationsChanged = 'acp:notifications-changed',
  AcpProfileChanged = 'acp:profile-changed',
  ComposerDraftAppend = 'composer:draft-append',
  RemotePairRequest = 'remote:pair-request',
  RemotePairResolved = 'remote:pair-resolved'
}

/**
 * Maps each command to its argument shape. `invoke(cmd, args)` infers
 * the args type and rejects mismatches at compile time. `void` for
 * commands that take no arguments.
 */
export interface TauriCommandArgs {
  [TauriCommand.BootSnapshot]: void
  [TauriCommand.GetTheme]: void
  [TauriCommand.GetKeymaps]: void
  [TauriCommand.GetWindowState]: void
  [TauriCommand.WindowToggle]: void
  [TauriCommand.DaemonRpc]: { method: string; params?: unknown }
  [TauriCommand.ReadFileForAttachment]: { path: string }
  [TauriCommand.SessionSubmit]: SubmitArgs
  [TauriCommand.SessionCancel]: CancelArgs
  [TauriCommand.AgentsList]: void
  [TauriCommand.ProfilesList]: void
  [TauriCommand.ProfileGet]: void
  [TauriCommand.ProfileSet]: { profileId: string }
  [TauriCommand.SessionList]: ListSessionsArgs
  [TauriCommand.SessionLoad]: LoadSessionArgs
  [TauriCommand.SessionsInfo]: SessionsInfoArgs
  [TauriCommand.PermissionReply]: PermissionReplyArgs
  [TauriCommand.InstancesList]: void
  [TauriCommand.InstancesFocus]: InstancesFocusArgs
  [TauriCommand.InstancesShutdown]: InstancesShutdownArgs
  [TauriCommand.InstancesRename]: InstancesRenameArgs
  [TauriCommand.InstanceRestart]: InstanceRestartArgs
  [TauriCommand.ModelsSet]: ModelsSetArgs
  [TauriCommand.ModesSet]: ModesSetArgs
  [TauriCommand.ConfigOptionSet]: ConfigOptionSetArgs
  [TauriCommand.EffortGet]: EffortGetArgs
  [TauriCommand.EffortSet]: EffortSetArgs
  [TauriCommand.EffortsList]: EffortsListArgs
  [TauriCommand.InstanceMeta]: InstanceMetaArgs
  [TauriCommand.InstanceSnapshotMeta]: InstanceSnapshotMetaArgs
  [TauriCommand.InstanceSnapshotChat]: InstanceSnapshotChatArgs
  [TauriCommand.InstanceSnapshotTerminals]: InstanceSnapshotTerminalsArgs
  [TauriCommand.InstanceSnapshotQueue]: { instanceId: string }
  [TauriCommand.McpsList]: McpsListArgs
  [TauriCommand.QueueList]: QueueListArgs
  [TauriCommand.QueueEdit]: QueueEditArgs
  [TauriCommand.QueueRemove]: QueueRemoveArgs
  [TauriCommand.QueueMove]: QueueMoveArgs
  [TauriCommand.QueueClear]: QueueClearArgs
  [TauriCommand.QueueDispatch]: QueueDispatchArgs
  [TauriCommand.NotificationsList]: void
  [TauriCommand.NotificationsGet]: NotificationsGetArgs
  [TauriCommand.NotificationsClear]: NotificationsClearArgs
  [TauriCommand.NotificationsClearAll]: void
  [TauriCommand.ResolveSpawnCwd]: { profileId?: string }
  [TauriCommand.CompletionQuery]: CompletionQueryArgs
  [TauriCommand.CompletionResolve]: CompletionResolveArgs
  [TauriCommand.CompletionCancel]: CompletionCancelArgs
  /**
   * Caller-supplied candidate ranking. Daemon ranks `candidates` against
   * `query` using nucleo (the same matcher path/ripgrep use). Drives
   * palette surfaces with bounded candidate sets — UI / Neovim plugin /
   * any future frontend share one ranking implementation.
   */
  [TauriCommand.CompletionRank]: { query: string; candidates: CandidateItem[] }
  [TauriCommand.GetCompletionConfig]: void
  [TauriCommand.SkillsReload]: void
  [TauriCommand.RemoteConfirmPair]: { pendingId: string; code: string }
  [TauriCommand.RemoteRejectPair]: { pendingId: string }
  [TauriCommand.RemotePendingPairs]: void
}

/** Maps each command to the response type Rust emits. `invoke(cmd)` infers the result. */
export interface TauriCommandResult {
  [TauriCommand.BootSnapshot]: BootSnapshot
  [TauriCommand.GetTheme]: Theme
  [TauriCommand.GetKeymaps]: KeymapsConfig
  [TauriCommand.GetWindowState]: WindowState
  [TauriCommand.WindowToggle]: boolean
  [TauriCommand.DaemonRpc]: unknown
  [TauriCommand.ReadFileForAttachment]: { path: string; body: string; binary: boolean; truncated: boolean }
  [TauriCommand.SessionSubmit]: SubmitResult
  [TauriCommand.SessionCancel]: CancelResult
  [TauriCommand.AgentsList]: { agents: AgentSummary[] }
  [TauriCommand.ProfilesList]: { profiles: ProfileSummary[] }
  [TauriCommand.ProfileGet]: string | null
  [TauriCommand.ProfileSet]: { profileId: string }
  [TauriCommand.SessionList]: { sessions: SessionSummary[] }
  [TauriCommand.SessionLoad]: void
  [TauriCommand.SessionsInfo]: SessionInfoResult
  [TauriCommand.PermissionReply]: void
  [TauriCommand.InstancesList]: { instances: InstanceListEntry[]; focusedId?: string }
  [TauriCommand.InstancesFocus]: { instanceId: string }
  [TauriCommand.InstancesShutdown]: { instanceId: string }
  [TauriCommand.InstancesRename]: InstancesRenameResult
  [TauriCommand.InstanceRestart]: InstanceRestartResult
  [TauriCommand.ModelsSet]: unknown
  [TauriCommand.ModesSet]: unknown
  [TauriCommand.ConfigOptionSet]: unknown
  [TauriCommand.EffortGet]: EffortGetResult
  [TauriCommand.EffortSet]: EffortGetResult
  [TauriCommand.EffortsList]: EffortsListResult
  [TauriCommand.InstanceMeta]: InstanceMetaSnapshot
  [TauriCommand.InstanceSnapshotMeta]: MetaSnapshot
  [TauriCommand.InstanceSnapshotChat]: ChatSnapshot
  [TauriCommand.InstanceSnapshotTerminals]: TerminalsSnapshot
  [TauriCommand.InstanceSnapshotQueue]: QueueListResult
  [TauriCommand.McpsList]: MCPListResult
  [TauriCommand.QueueList]: QueueListResult
  [TauriCommand.QueueEdit]: QueueEditResult
  [TauriCommand.QueueRemove]: QueueRemoveResult
  [TauriCommand.QueueMove]: QueueMoveResult
  [TauriCommand.QueueClear]: QueueClearResult
  [TauriCommand.QueueDispatch]: QueueDispatchResult
  [TauriCommand.NotificationsList]: NotificationsSnapshot
  [TauriCommand.NotificationsGet]: NotificationsGetResult
  [TauriCommand.NotificationsClear]: { cleared: boolean }
  [TauriCommand.NotificationsClearAll]: { cleared: boolean }
  /** Rust ships `cwd: null` (not absent) when the resolved
   *  profile has no cwd after patches — caller coerces `null` to
   *  `undefined` at the seed site. */
  [TauriCommand.ResolveSpawnCwd]: { cwd: string | null }
  [TauriCommand.CompletionQuery]: CompletionQueryResponse
  [TauriCommand.CompletionResolve]: CompletionResolveResponse
  [TauriCommand.CompletionCancel]: CompletionCancelResponse
  [TauriCommand.CompletionRank]: CompletionQueryResponse
  [TauriCommand.GetCompletionConfig]: CompletionConfigSnapshot
  [TauriCommand.SkillsReload]: { count: number }
  [TauriCommand.RemoteConfirmPair]: { confirmed: boolean }
  [TauriCommand.RemoteRejectPair]: void
  [TauriCommand.RemotePendingPairs]: RemotePendingPair[]
}

/** Maps each event to its payload type. `listen(ev, cb)` infers `cb`'s arg. */
export interface TauriEventPayload {
  [TauriEvent.AcpTranscript]: TranscriptEventPayload
  [TauriEvent.AcpInstanceState]: InstanceStateEventPayload
  [TauriEvent.AcpPermissionRequest]: PermissionRequestEventPayload
  [TauriEvent.AcpPermissionResolved]: AcpPermissionResolvedPayload
  [TauriEvent.AcpTurnStarted]: TurnStartedEventPayload
  [TauriEvent.AcpTurnEnded]: TurnEndedEventPayload
  [TauriEvent.AcpTerminal]: TerminalEventPayload
  [TauriEvent.AcpInstancesChanged]: InstancesChangedEventPayload
  [TauriEvent.AcpInstancesFocused]: InstancesFocusedEventPayload
  [TauriEvent.AcpInstanceRenamed]: InstanceRenamedEventPayload
  [TauriEvent.AcpSessionInfoUpdate]: SessionInfoUpdateEventPayload
  [TauriEvent.AcpCurrentModeUpdate]: CurrentModeUpdateEventPayload
  [TauriEvent.AcpUsageUpdate]: UsageUpdateEventPayload
  [TauriEvent.AcpConfigOptionsUpdate]: ConfigOptionsUpdateEventPayload
  [TauriEvent.AcpInstanceMeta]: InstanceMetaEventPayload
  [TauriEvent.AcpSystemPromptInjected]: SystemPromptInjectedEventPayload
  [TauriEvent.AcpQueueChanged]: AcpQueueChangedPayload
  [TauriEvent.AcpNotificationsChanged]: NotificationsChangedEventPayload
  [TauriEvent.AcpProfileChanged]: { profileId: string }
  [TauriEvent.ComposerDraftAppend]: ComposerDraftAppendEventPayload
  [TauriEvent.RemotePairRequest]: RemotePairRequestEventPayload
  [TauriEvent.RemotePairResolved]: RemotePairResolvedEventPayload
}
