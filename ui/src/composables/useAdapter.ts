import {
  invoke,
  TauriCommand,
  type AgentSummary,
  type CancelResult,
  type ProfileSummary,
  type SubmitResult
} from '@ipc'

import { ToastTone } from '@components/types'

import { pushToast } from './useToasts'

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
    return invoke(TauriCommand.SessionSubmit, {
      text: options.text,
      agentId: options.agentId,
      profileId: options.profileId
    })
  }

  async function cancel(agentId?: string): Promise<CancelResult> {
    const result = await invoke(TauriCommand.SessionCancel, { agentId })
    if (result.cancelled) {
      pushToast(ToastTone.Warn, 'turn cancelled')
    }
    return result
  }

  async function agentsList(): Promise<AgentSummary[]> {
    const r = await invoke(TauriCommand.AgentsList)

    return r.agents
  }

  async function profilesList(): Promise<ProfileSummary[]> {
    const r = await invoke(TauriCommand.ProfilesList)

    return r.profiles
  }

  return {
    submit,
    cancel,
    agentsList,
    profilesList
  }
}
