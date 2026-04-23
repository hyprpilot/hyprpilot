import { invoke } from '@ipc'

import { type InstanceId } from './useActiveInstance'

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
 * Thin submit/cancel/list surface. Permission events stream via
 * `useSessionStream` into `usePermissions`; transcript + state via
 * `useTranscript` / `useSessionStream`.
 */
export function useAdapter() {
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
    submit,
    cancel,
    agentsList,
    profilesList
  }
}
