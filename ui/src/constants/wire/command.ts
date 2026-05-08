/**
 * Wire-contract registry for every Tauri `invoke` command and `listen`
 * event the UI consumes. Mirrors the Rust side: `invoke_handler![...]`
 * in `src-tauri/src/daemon/mod.rs` and the `app.emit(...)` adapter
 * event emitters. Raw string literals at call sites are banned —
 * typos would only surface at runtime. The `*Result` / `*Payload`
 * interfaces below pick the response / event type off the command or
 * event name so `invoke` / `listen` infer it automatically.
 */

import type { GitStatus } from '@interfaces/ui/header'
import type {
  CandidateItem,
  CompletionCancelArgs,
  CompletionCancelResponse,
  CompletionQueryArgs,
  CompletionQueryResponse,
  CompletionResolveArgs,
  CompletionResolveResponse
} from '@interfaces/wire/completion'
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
import type { AcpPermissionResolvedPayload } from '@interfaces/wire/permission-resolved'
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
  GetHomeDir = 'get_home_dir',
  GetDaemonCwd = 'get_daemon_cwd',
  GetGitStatus = 'get_git_status',
  PathsResolve = 'paths_resolve',
  DaemonRpc = 'daemon_rpc',
  ReadFileForAttachment = 'read_file_for_attachment',
  SessionSubmit = 'session_submit',
  SessionCancel = 'session_cancel',
  AgentsList = 'agents_list',
  ProfilesList = 'profiles_list',
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
  InstanceMeta = 'instance_meta',
  InstanceSnapshotMeta = 'instance_snapshot_meta',
  InstanceSnapshotChat = 'instance_snapshot_chat',
  InstanceSnapshotTerminals = 'instance_snapshot_terminals',
  McpsList = 'mcps_list',
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
  [TauriCommand.GetHomeDir]: void
  [TauriCommand.GetDaemonCwd]: void
  [TauriCommand.GetGitStatus]: { path: string }
  [TauriCommand.PathsResolve]: { raw: string; cwdBase?: string }
  [TauriCommand.DaemonRpc]: { method: string; params?: unknown }
  [TauriCommand.ReadFileForAttachment]: { path: string }
  [TauriCommand.SessionSubmit]: SubmitArgs
  [TauriCommand.SessionCancel]: CancelArgs
  [TauriCommand.AgentsList]: void
  [TauriCommand.ProfilesList]: void
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
  [TauriCommand.InstanceMeta]: InstanceMetaArgs
  [TauriCommand.InstanceSnapshotMeta]: InstanceSnapshotMetaArgs
  [TauriCommand.InstanceSnapshotChat]: InstanceSnapshotChatArgs
  [TauriCommand.InstanceSnapshotTerminals]: InstanceSnapshotTerminalsArgs
  [TauriCommand.McpsList]: McpsListArgs
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
  [TauriCommand.GetHomeDir]: string
  [TauriCommand.GetDaemonCwd]: string
  [TauriCommand.GetGitStatus]: GitStatus | null
  /**
   * Captain-typed → absolute resolution. `null` when the input is empty
   * or relative-with-no-cwd-base. The daemon owns `${VAR}` interpolation
   * (process env), `~` expansion ($HOME), and relative→absolute join
   * (cwd_base) so the webview doesn't re-derive logic that needs OS
   * access.
   */
  [TauriCommand.PathsResolve]: string | null
  [TauriCommand.DaemonRpc]: unknown
  [TauriCommand.ReadFileForAttachment]: { path: string; body: string; binary: boolean; truncated: boolean }
  [TauriCommand.SessionSubmit]: SubmitResult
  [TauriCommand.SessionCancel]: CancelResult
  [TauriCommand.AgentsList]: { agents: AgentSummary[] }
  [TauriCommand.ProfilesList]: { profiles: ProfileSummary[] }
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
  [TauriCommand.InstanceMeta]: InstanceMetaSnapshot
  [TauriCommand.InstanceSnapshotMeta]: MetaSnapshot
  [TauriCommand.InstanceSnapshotChat]: ChatSnapshot
  [TauriCommand.InstanceSnapshotTerminals]: TerminalsSnapshot
  [TauriCommand.McpsList]: MCPListResult
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

/**
 * Snapshot of the daemon's `[completion]` config block. Returned by
 * the boot-time `get_completion_config` Tauri command. UI uses
 * `ripgrep.debounceMs` to slow auto-trigger queries since ripgrep
 * walks the cwd's file tree per call.
 */
export interface CompletionConfigSnapshot {
  ripgrep: {
    auto: boolean
    debounceMs: number
    minPrefix: number
  }
}

/**
 * Aggregated boot payload. One `invoke('boot_snapshot')` returns
 * everything the loading screen needs before it can drop. Replaces
 * six sequential `await invoke(...)` round-trips — particularly
 * load-bearing on the remote bridge where each round-trip rides the
 * same WS, so the captain spent up to 6× RTT staring at the loader.
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
  [TauriEvent.ComposerDraftAppend]: ComposerDraftAppendEventPayload
  [TauriEvent.RemotePairRequest]: RemotePairRequestEventPayload
  [TauriEvent.RemotePairResolved]: RemotePairResolvedEventPayload
}

/**
 * Payload of `remote:pair-request` — emitted on every WS upgrade
 * the daemon receives from a phone (or any browser) hitting the
 * remote bridge. Carries BOTH codes: the desktop renders its own
 * (`desktopCode`) as QR + words and expects the captain to present
 * the device's code (`deviceCode`) — typed manually, or scanned
 * from the device's QR. Asymmetric codes are the whole point of
 * the pairing: presenting the same code visible on the same screen
 * proves nothing.
 */
export interface RemotePairRequestEventPayload {
  pendingId: string
  /** Code rendered on the connecting device — desktop's expected input. */
  deviceCode: string
  /** Code rendered on the desktop modal — device's expected input. */
  desktopCode: string
  remoteAddr: string
}

/**
 * Payload of `remote:pair-resolved` — emitted whenever a pending
 * pair transitions out of `pending` (confirmed by either side, or
 * rejected via timeout / captain-reject / attempt-cap / connection
 * drop). The desktop modal listens for this and clears its state
 * the moment it lands; without it the modal would stay open after
 * the device side authenticates first (captain scanned the desktop's
 * QR with the phone).
 */
export interface RemotePairResolvedEventPayload {
  pendingId: string
  outcome: 'confirmed' | 'rejected'
}

/**
 * Snapshot row from `remote_pending_pairs`. Diagnostic surface for
 * "queue of waiting devices" UX.
 */
export interface RemotePendingPair {
  pendingId: string
  remoteAddr: string
  expiresInSeconds: number
}
