/**
 * Wire-contract registry for every Tauri `invoke` command and `listen`
 * event the UI consumes. Mirrors the Rust side: `invoke_handler![...]`
 * in `src-tauri/src/daemon/mod.rs` and the `app.emit(...)` adapter
 * event emitters. Raw string literals at call sites are banned —
 * typos would only surface at runtime. The `*Result` / `*Payload`
 * interfaces below pick the response / event type off the command or
 * event name so `invoke` / `listen` infer it automatically.
 */

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
  InstancesShutdownArgs,
  ListSessionsArgs,
  LoadSessionArgs,
  McpsListArgs,
  McpsSetArgs,
  MCPListResult,
  MCPSetResult,
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
import type {
  CurrentModeUpdateEventPayload,
  InstanceMetaEventPayload,
  InstanceStateEventPayload,
  InstancesChangedEventPayload,
  InstancesFocusedEventPayload,
  PermissionRequestEventPayload,
  SessionInfoUpdateEventPayload,
  TerminalEventPayload,
  TranscriptEventPayload,
  TurnEndedEventPayload,
  TurnStartedEventPayload
} from '@interfaces/wire/event'
import type { KeymapsConfig } from '@interfaces/wire/keymap'
import type { Theme } from '@interfaces/wire/theme'
import type { WindowState } from '@interfaces/wire/window'

export enum TauriCommand {
  GetTheme = 'get_theme',
  GetKeymaps = 'get_keymaps',
  GetWindowState = 'get_window_state',
  GetHomeDir = 'get_home_dir',
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
  InstanceRestart = 'instance_restart',
  ModelsSet = 'models_set',
  ModesSet = 'modes_set',
  InstanceMeta = 'instance_meta',
  McpsList = 'mcps_list',
  McpsSet = 'mcps_set'
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
  AcpSessionInfoUpdate = 'acp:session-info-update',
  AcpCurrentModeUpdate = 'acp:current-mode-update',
  AcpInstanceMeta = 'acp:instance-meta'
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
  [TauriCommand.GetHomeDir]: void
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
  [TauriCommand.InstanceRestart]: InstanceRestartArgs
  [TauriCommand.ModelsSet]: ModelsSetArgs
  [TauriCommand.ModesSet]: ModesSetArgs
  [TauriCommand.InstanceMeta]: InstanceMetaArgs
  [TauriCommand.McpsList]: McpsListArgs
  [TauriCommand.McpsSet]: McpsSetArgs
}

/** Maps each command to the response type Rust emits. `invoke(cmd)` infers the result. */
export interface TauriCommandResult {
  [TauriCommand.GetTheme]: Theme
  [TauriCommand.GetKeymaps]: KeymapsConfig
  [TauriCommand.GetWindowState]: WindowState
  [TauriCommand.GetHomeDir]: string
  [TauriCommand.SessionSubmit]: SubmitResult
  [TauriCommand.SessionCancel]: CancelResult
  [TauriCommand.AgentsList]: { agents: AgentSummary[] }
  [TauriCommand.ProfilesList]: { profiles: ProfileSummary[] }
  [TauriCommand.SessionList]: { sessions: SessionSummary[] }
  [TauriCommand.SessionLoad]: void
  [TauriCommand.SessionsInfo]: SessionInfoResult
  [TauriCommand.PermissionReply]: void
  [TauriCommand.InstancesList]: { instances: InstanceListEntry[] }
  [TauriCommand.InstancesFocus]: { focusedId: string }
  [TauriCommand.InstancesShutdown]: { id: string }
  [TauriCommand.InstanceRestart]: InstanceRestartResult
  [TauriCommand.ModelsSet]: unknown
  [TauriCommand.ModesSet]: unknown
  [TauriCommand.InstanceMeta]: InstanceMetaSnapshot
  [TauriCommand.McpsList]: MCPListResult
  [TauriCommand.McpsSet]: MCPSetResult
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
  [TauriEvent.AcpSessionInfoUpdate]: SessionInfoUpdateEventPayload
  [TauriEvent.AcpCurrentModeUpdate]: CurrentModeUpdateEventPayload
  [TauriEvent.AcpInstanceMeta]: InstanceMetaEventPayload
}
