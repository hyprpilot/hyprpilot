/**
 * Wire-contract registry for every Tauri `invoke` command and `listen`
 * event the UI consumes. Mirrors the Rust side: `invoke_handler![...]`
 * in `src-tauri/src/daemon/mod.rs` and the `app.emit(...)` / adapter
 * event emitters. Raw string literals at call sites are banned —
 * typos would only surface at runtime. The `*Result` / `*Payload`
 * interfaces below pick the response / event type off the command or
 * event name so `invoke` / `listen` infer it automatically.
 */

import type {
  AgentSummary,
  CancelResult,
  GtkFont,
  InstanceListEntry,
  InstanceRestartResult,
  InstancesChangedEventPayload,
  InstancesFocusedEventPayload,
  InstanceStateEventPayload,
  KeymapsConfig,
  MCPListResult,
  MCPSetResult,
  PermissionRequestEventPayload,
  ProfileSummary,
  SessionInfoResult,
  SessionSummary,
  SkillBody,
  SkillSummary,
  SlashCommand,
  SubmitResult,
  TerminalEventPayload,
  Theme,
  TranscriptEventPayload,
  TurnEndedEventPayload,
  TurnStartedEventPayload,
  WindowState
} from './types'

export enum TauriCommand {
  GetTheme = 'get_theme',
  GetKeymaps = 'get_keymaps',
  GetWindowState = 'get_window_state',
  GetGtkFont = 'get_gtk_font',
  GetHomeDir = 'get_home_dir',
  SessionSubmit = 'session_submit',
  SessionCancel = 'session_cancel',
  AgentsList = 'agents_list',
  CommandsList = 'commands_list',
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
  SkillsList = 'skills_list',
  SkillsGet = 'skills_get',
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
  AcpInstancesFocused = 'acp:instances-focused'
}

/** Maps each command to the response type Rust emits. `invoke(cmd)` infers the result. */
export interface TauriCommandResult {
  [TauriCommand.GetTheme]: Theme
  [TauriCommand.GetKeymaps]: KeymapsConfig
  [TauriCommand.GetWindowState]: WindowState
  [TauriCommand.GetGtkFont]: GtkFont | null
  [TauriCommand.GetHomeDir]: string | null
  [TauriCommand.SessionSubmit]: SubmitResult
  [TauriCommand.SessionCancel]: CancelResult
  [TauriCommand.AgentsList]: { agents: AgentSummary[] }
  [TauriCommand.CommandsList]: { commands: SlashCommand[] }
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
  [TauriCommand.SkillsList]: { skills: SkillSummary[] }
  [TauriCommand.SkillsGet]: SkillBody
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
}
