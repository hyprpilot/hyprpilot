import { onBeforeUnmount, ref } from 'vue'

import { invoke, listen, type UnlistenFn } from '@ipc'

import { type InstanceId } from './useActiveInstance'
import { type PermissionOptionView } from './useSessionStream'

export interface PermissionRequestEvent {
  agent_id: string
  session_id: string
  instance_id?: InstanceId
  options: PermissionOptionView[]
}

export interface SubmitResult {
  accepted: boolean
  agent_id: string
  profile_id?: string
  session_id?: string
  instance_id?: InstanceId
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

/**
 * Thin submit/cancel/list surface + a bound `lastPermission` ref.
 * Event demuxing (`acp:transcript` / `acp:instance-state`) lives in
 * `useSessionStream`; permission events stay here because the chat
 * shell consumes them directly for the permission stack.
 */
export function useAdapter() {
  const lastPermission = ref<PermissionRequestEvent>()

  const unlisteners: UnlistenFn[] = []

  async function bind() {
    unlisteners.push(
      await listen<PermissionRequestEvent>('acp:permission-request', (e) => {
        lastPermission.value = e.payload
      })
    )
  }

  function unbind() {
    for (const u of unlisteners) {
      u()
    }
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
    lastPermission,
    bind,
    unbind,
    submit,
    cancel,
    agentsList,
    profilesList
  }
}
