import {
  invoke,
  TauriCommand,
  type AgentSummary,
  type Attachment,
  type CancelResult,
  type ProfileSummary,
  type SubmitResult
} from '@ipc'

import { ToastTone } from '@components'

import { pushToast } from '../ui-state/use-toasts'

export interface SubmitOptions {
  text: string
  /**
   * UUID of the instance this prompt targets. Omit to mint a fresh
   * instance server-side; provide to route a follow-up to a live one
   * (or to adopt-on-first-sight a client-generated UUID so the
   * webview can push its user turn optimistically before the RPC
   * round-trip completes).
   */
  instanceId?: string
  agentId?: string
  profileId?: string
  /**
   * First-class skill / resource attachments delivered alongside
   * `text`. Backend (K-268) maps each entry onto an ACP
   * `ContentBlock::Resource` prepended before the prompt text block.
   */
  attachments?: Attachment[]
}

export interface CancelOptions {
  /** UUID of the instance to cancel. Preferred over `agentId`. */
  instanceId?: string
  agentId?: string
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
      instanceId: options.instanceId,
      agentId: options.agentId,
      profileId: options.profileId,
      attachments: options.attachments ?? []
    })
  }

  async function cancel(options: CancelOptions = {}): Promise<CancelResult> {
    const result = await invoke(TauriCommand.SessionCancel, {
      instanceId: options.instanceId,
      agentId: options.agentId
    })
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
