import { pushToast } from '../ui-state/use-toasts'
import { ToastTone } from '@components'
import { invoke, TauriCommand, type AgentSummary, type Attachment, type CancelResult, type ProfileSummary, type SubmitResult } from '@ipc'

export interface SubmitOptions {
  text: string
  /**
   * UUID of the instance this prompt targets. Omit to spawn fresh
   * server-side — the daemon mints via `InstanceKey::new_v4()` and
   * returns the issued id on `SubmitResult.instanceId`. UI-side
   * minting is intentionally not the contract: every per-instance
   * surface (ChatViewport `:key`, snapshot cache, palette listing)
   * flips off a value that must exist server-side, and a
   * pre-emptive UI mint races the spawn task — the snapshot RPC for
   * the freshly-minted id returns "not found" before the actor
   * lands, the InfiniteQuery enters error state, and the
   * transcript-patcher silently drops live frames against the
   * empty cache. Daemon-issued ids land synchronously with the
   * spawn so the round-trip is the right place to learn the id.
   */
  instanceId?: string
  agentId?: string
  profileId?: string
  /**
   * First-class skill / resource attachments delivered alongside
   * `text`. Each entry maps onto an ACP `ContentBlock::Resource`
   * prepended before the prompt text block.
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
export interface UseAdapterApi {
  submit: (options: SubmitOptions) => Promise<SubmitResult>
  cancel: (options?: CancelOptions) => Promise<CancelResult>
  agentsList: () => Promise<AgentSummary[]>
  profilesList: () => Promise<ProfileSummary[]>
}

export function useAdapter(): UseAdapterApi {
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
