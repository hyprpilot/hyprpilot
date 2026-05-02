import { invoke as tauriInvoke } from '@tauri-apps/api/core'
import { listen as tauriListen, type EventCallback, type UnlistenFn } from '@tauri-apps/api/event'

import { TauriCommand, TauriEvent, type TauriCommandArgs, type TauriCommandResult, type TauriEventPayload } from '@constants/wire'

/**
 * Typed `invoke` wrapper. Args are inferred from `TauriCommandArgs[K]`
 * — no `Record<string, unknown>` escape hatch — so call sites get
 * compile-time validation of every arg shape. Every backend rejection
 * logs a structured `error`-level entry tagged with the command name +
 * args BEFORE the rejection propagates — single audit trail for
 * "what backend call failed, when, with what payload" so callers don't
 * each need to remember to log. Callers still toast user-facing errors
 * themselves; this is observability, not UX.
 *
 * Commands declared with `void` args don't take a second positional
 * argument; commands with non-void args require the typed object.
 * TypeScript narrows on `K`, so the right overload picks per call.
 */
export async function invoke<K extends TauriCommand>(
  ...args: TauriCommandArgs[K] extends void ? [command: K] : [command: K, args: TauriCommandArgs[K]]
): Promise<TauriCommandResult[K]> {
  const [command, payload] = args

  try {
    return await tauriInvoke<TauriCommandResult[K]>(command, payload as Record<string, unknown> | undefined)
  } catch(err) {
    // Lazy-import to avoid an `@lib` <-> `@ipc` cyclic dep at module
    // load time. The runtime cost is negligible — once-per-error.
    const { log } = await import('@lib')

    log.error('invoke failed', { command, args: payload }, err)
    throw err
  }
}

export async function listen<K extends TauriEvent>(event: K, cb: EventCallback<TauriEventPayload[K]>): Promise<UnlistenFn> {
  return tauriListen<TauriEventPayload[K]>(event, cb)
}

export type { EventCallback, UnlistenFn }
