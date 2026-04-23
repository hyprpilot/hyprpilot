import { onBeforeUnmount, reactive, ref } from 'vue'

import { invoke, listen, type UnlistenFn } from '@ipc'

export enum EventKind {
  Transcript = 'transcript',
  State = 'state',
  PermissionRequest = 'permission_request'
}

export enum SessionState {
  Starting = 'starting',
  Running = 'running',
  Ended = 'ended',
  Error = 'error'
}

export interface PermissionOptionView {
  option_id: string
  name: string
  kind: string
}

export interface TranscriptEvent {
  kind: EventKind.Transcript
  agent_id: string
  session_id: string
  update: Record<string, unknown>
}

export interface PermissionRequestEvent {
  kind: EventKind.PermissionRequest
  agent_id: string
  session_id: string
  options: PermissionOptionView[]
}

export interface SessionStateEvent {
  kind: EventKind.State
  agent_id: string
  session_id?: string
  state: SessionState
}

export interface SubmitResult {
  accepted: boolean
  agent_id: string
  profile_id?: string
  session_id?: string
}

export interface CancelResult {
  cancelled: boolean
  reason?: string
}

export interface AgentSummary {
  id: string
  provider: string
  is_default: boolean
}

export interface ProfileSummary {
  id: string
  agent: string
  model?: string
  has_prompt: boolean
  is_default: boolean
}

export interface SubmitOptions {
  text: string
  agentId?: string
  profileId?: string
}

export function useAdapter() {
  const transcript = reactive<TranscriptEvent[]>([])
  const state = ref<SessionStateEvent>()
  const lastPermission = ref<PermissionRequestEvent>()

  const unlisteners: UnlistenFn[] = []

  async function bind() {
    unlisteners.push(
      await listen<TranscriptEvent>('acp:transcript', (e) => {
        transcript.push(e.payload)
      }),
      await listen<SessionStateEvent>('acp:instance-state', (e) => {
        state.value = e.payload
      }),
      await listen<PermissionRequestEvent>('acp:permission-request', (e) => {
        lastPermission.value = e.payload
      })
    )
  }

  function unbind() {
    for (const u of unlisteners) u()
    unlisteners.length = 0
  }

  onBeforeUnmount(unbind)

  async function submit(options: SubmitOptions): Promise<SubmitResult> {
    return invoke<SubmitResult>('acp_submit', {
      text: options.text,
      agentId: options.agentId,
      profileId: options.profileId
    })
  }

  async function cancel(agentId?: string): Promise<CancelResult> {
    return invoke<CancelResult>('acp_cancel', { agentId })
  }

  async function agentsList(): Promise<AgentSummary[]> {
    const r = await invoke<{ agents: AgentSummary[] }>('agents_list')

    return r.agents
  }

  async function profilesList(): Promise<ProfileSummary[]> {
    const r = await invoke<{ profiles: ProfileSummary[] }>('profiles_list')

    return r.profiles
  }

  return {
    transcript,
    state,
    lastPermission,
    bind,
    unbind,
    submit,
    cancel,
    agentsList,
    profilesList
  }
}
