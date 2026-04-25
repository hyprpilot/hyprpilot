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
  InstanceStateEventPayload,
  KeymapsConfig,
  MCPListResult,
  MCPSetResult,
  PermissionRequestEventPayload,
  ProfileSummary,
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
  PermissionReply = 'permission_reply',
  // K-265: switch the active model / mode on the addressed instance.
  // Both Tauri commands stub past the membership check at the adapter
  // (matching the `models/set` / `modes/set` wire shape) until K-251
  // wires the runtime side; UI surfaces the error via toast.
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
  AcpTerminal = 'acp:terminal'
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
  [TauriCommand.PermissionReply]: void
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
}
