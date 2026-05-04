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
  CurrentModeUpdateEventPayload,
  InstanceMetaEventPayload,
  InstanceStateEventPayload,
  InstancesChangedEventPayload,
  InstancesFocusedEventPayload,
  InstanceRenamedEventPayload,
  PermissionRequestEventPayload,
  SessionInfoUpdateEventPayload,
  TerminalEventPayload,
  TranscriptEventPayload,
  TurnEndedEventPayload,
  TurnStartedEventPayload
} from '@interfaces/wire/event'
import type { KeymapsConfig } from '@interfaces/wire/keymap'
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
  PermissionsTrustSnapshot = 'permissions_trust_snapshot',
  PermissionsTrustForget = 'permissions_trust_forget',
  InstancesList = 'instances_list',
  InstancesFocus = 'instances_focus',
  InstancesShutdown = 'instances_shutdown',
  InstancesRename = 'instances_rename',
  InstanceRestart = 'instance_restart',
  ModelsSet = 'models_set',
  ModesSet = 'modes_set',
  InstanceMeta = 'instance_meta',
  McpsList = 'mcps_list',
  CompletionQuery = 'completion_query',
  CompletionResolve = 'completion_resolve',
  CompletionCancel = 'completion_cancel',
  CompletionRank = 'completion_rank',
  GetCompletionConfig = 'get_completion_config',
  SkillsReload = 'skills_reload'
}

export enum TauriEvent {
  AcpTranscript = 'acp:transcript',
  AcpPermissionRequest = 'acp:permission-request',
  AcpInstanceState = 'acp:instance-state',
  AcpTurnStarted = 'acp:turn-started',
  AcpTurnEnded = 'acp:turn-ended',
  AcpTerminal = 'acp:terminal',
  AcpInstancesChanged = 'acp:instances-changed',
  AcpInstancesFocused = 'acp:instances-focused',
  AcpInstanceRenamed = 'acp:instance-renamed',
  AcpSessionInfoUpdate = 'acp:session-info-update',
  AcpCurrentModeUpdate = 'acp:current-mode-update',
  AcpInstanceMeta = 'acp:instance-meta',
  ComposerDraftAppend = 'composer:draft-append'
}

/**
 * Maps each command to its argument shape. `invoke(cmd, args)` infers
 * the args type and rejects mismatches at compile time. `void` for
 * commands that take no arguments.
 */
export interface TauriCommandArgs {
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
  [TauriCommand.PermissionsTrustSnapshot]: { instanceId: string }
  [TauriCommand.PermissionsTrustForget]: { instanceId: string; tool: string }
  [TauriCommand.InstancesList]: void
  [TauriCommand.InstancesFocus]: InstancesFocusArgs
  [TauriCommand.InstancesShutdown]: InstancesShutdownArgs
  [TauriCommand.InstancesRename]: InstancesRenameArgs
  [TauriCommand.InstanceRestart]: InstanceRestartArgs
  [TauriCommand.ModelsSet]: ModelsSetArgs
  [TauriCommand.ModesSet]: ModesSetArgs
  [TauriCommand.InstanceMeta]: InstanceMetaArgs
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
}

/** Maps each command to the response type Rust emits. `invoke(cmd)` infers the result. */
export interface TauriCommandResult {
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
  [TauriCommand.PermissionsTrustSnapshot]: { entries: { tool: string; decision: 'allow' | 'deny' }[] }
  [TauriCommand.PermissionsTrustForget]: void
  [TauriCommand.InstancesList]: { instances: InstanceListEntry[] }
  [TauriCommand.InstancesFocus]: { focusedId: string }
  [TauriCommand.InstancesShutdown]: { id: string }
  [TauriCommand.InstancesRename]: InstancesRenameResult
  [TauriCommand.InstanceRestart]: InstanceRestartResult
  [TauriCommand.ModelsSet]: unknown
  [TauriCommand.ModesSet]: unknown
  [TauriCommand.InstanceMeta]: InstanceMetaSnapshot
  [TauriCommand.McpsList]: MCPListResult
  [TauriCommand.CompletionQuery]: CompletionQueryResponse
  [TauriCommand.CompletionResolve]: CompletionResolveResponse
  [TauriCommand.CompletionCancel]: CompletionCancelResponse
  [TauriCommand.CompletionRank]: CompletionQueryResponse
  [TauriCommand.GetCompletionConfig]: CompletionConfigSnapshot
  [TauriCommand.SkillsReload]: { count: number }
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

/** Maps each event to its payload type. `listen(ev, cb)` infers `cb`'s arg. */
export interface TauriEventPayload {
  [TauriEvent.AcpTranscript]: TranscriptEventPayload
  [TauriEvent.AcpInstanceState]: InstanceStateEventPayload
  [TauriEvent.AcpPermissionRequest]: PermissionRequestEventPayload
  [TauriEvent.AcpTurnStarted]: TurnStartedEventPayload
  [TauriEvent.AcpTurnEnded]: TurnEndedEventPayload
  [TauriEvent.AcpTerminal]: TerminalEventPayload
  [TauriEvent.AcpInstancesChanged]: InstancesChangedEventPayload
  [TauriEvent.AcpInstancesFocused]: InstancesFocusedEventPayload
  [TauriEvent.AcpInstanceRenamed]: InstanceRenamedEventPayload
  [TauriEvent.AcpSessionInfoUpdate]: SessionInfoUpdateEventPayload
  [TauriEvent.AcpCurrentModeUpdate]: CurrentModeUpdateEventPayload
  [TauriEvent.AcpInstanceMeta]: InstanceMetaEventPayload
  [TauriEvent.ComposerDraftAppend]: ComposerDraftAppendEventPayload
}
