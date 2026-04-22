import { invoke } from '@tauri-apps/api/core'
import { type UnlistenFn, listen } from '@tauri-apps/api/event'
import { onBeforeUnmount, reactive, ref } from 'vue'

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

export function useAcpAgent() {
  const transcript = reactive<TranscriptEvent[]>([])
  const state = ref<SessionStateEvent>()
  const lastPermission = ref<PermissionRequestEvent>()

  const unlisteners: UnlistenFn[] = []

  async function bind() {
    unlisteners.push(
      await listen<TranscriptEvent>('acp:transcript', (e) => {
        transcript.push(e.payload)
      }),
      await listen<SessionStateEvent>('acp:session-state', (e) => {
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

  async function submit(text: string, agentId?: string): Promise<SubmitResult> {
    return invoke<SubmitResult>('acp_submit', { text, agentId })
  }

  async function cancel(agentId?: string): Promise<CancelResult> {
    return invoke<CancelResult>('acp_cancel', { agentId })
  }

  async function agentsList(): Promise<AgentSummary[]> {
    const r = await invoke<{ agents: AgentSummary[] }>('agents_list')

    return r.agents
  }

  return {
    transcript,
    state,
    lastPermission,
    bind,
    unbind,
    submit,
    cancel,
    agentsList
  }
}
