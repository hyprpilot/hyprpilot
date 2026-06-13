import type { QueryClient } from '@tanstack/vue-query'

import { emptyPartialChatData, snapshotChatKey } from './chat-cache'
import { setSessionRestoring } from './use-session-info'
import type { InstanceId } from '../chrome/use-active-instance'
import { pushToast } from '../ui-state/use-toasts'
import { ToastTone } from '@components'
import { invoke, TauriCommand } from '@ipc'
import { appQueryClient, log } from '@lib'

type SessionLifecycleCommand = TauriCommand.SessionLoad | TauriCommand.SessionFork

export interface StartSessionLifecycleArgs {
  command: SessionLifecycleCommand
  sessionId: string
  agentId?: string
  profileId?: string
  cwd?: string
  target?: InstanceId
  queryClient?: QueryClient
  okToast: string
  errToastPrefix: string
  logLabel: string
}

export interface StartSessionLifecycleResult {
  ok: boolean
  target: InstanceId
  err?: string
}

function seedChatCache(queryClient: QueryClient | undefined, target: InstanceId): void {
  queryClient?.setQueryData(snapshotChatKey(target), emptyPartialChatData())
}

export async function startSessionLifecycle(args: StartSessionLifecycleArgs): Promise<StartSessionLifecycleResult> {
  const target = args.target ?? crypto.randomUUID()

  setSessionRestoring(target, true)
  seedChatCache(args.queryClient ?? appQueryClient(), target)

  const payload = {
    agentId: args.agentId,
    profileId: args.profileId,
    sessionId: args.sessionId,
    instanceId: target,
    cwd: args.cwd
  }

  try {
    if (args.command === TauriCommand.SessionLoad) {
      await invoke(TauriCommand.SessionLoad, payload)
    } else {
      await invoke(TauriCommand.SessionFork, payload)
    }
    pushToast(ToastTone.Ok, args.okToast)

    return {
      ok: true,
      target
    }
  } catch(err) {
    const message = String(err)

    log.warn(args.logLabel, {
      sessionId: args.sessionId,
      target,
      err: message
    })
    pushToast(ToastTone.Err, `${args.errToastPrefix}: ${message}`)
    setSessionRestoring(target, false)

    return {
      ok: false,
      target,
      err: message
    }
  }
}
