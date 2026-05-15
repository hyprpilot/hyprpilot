import { invoke as tauriInvoke } from '@tauri-apps/api/core'
import { listen as tauriListen, type EventCallback, type UnlistenFn } from '@tauri-apps/api/event'

import { remoteInvoke, remoteListen, isRemoteHost } from './remote-bridge'
import { TauriCommand, TauriEvent, type TauriCommandArgs, type TauriCommandResult, type TauriEventPayload } from '@constants/wire'
// Import the leaf `@lib/log` directly, NOT the `@lib` barrel. The
// barrel re-exports `./markdown`, which pulls `@ipc` for its
// `read_file_for_attachment` hydrator — that closes the loop
// `@ipc/bridge → @lib → @lib/markdown → @ipc`. The leaf has zero
// `@ipc` deps so it's safe at module-load time. Beyond breaking the
// cycle this also lets Rolldown chunk `@lib/markdown` (shiki + the
// markdown pipeline) away from the entry bundle — see vite.config.ts
// `codeSplitting.groups`.
import { log } from '@lib/log'

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
 * Two transports behind one signature:
 * - **Tauri** (default) — `window.__TAURI_INTERNALS__` is present;
 *   calls flow through `@tauri-apps/api/core::invoke`.
 * - **Remote WS** (browser hitting the `/` endpoint of the daemon's
 *   axum bridge) — no Tauri internals; calls flow through a single
 *   duplex WS that mirrors the unix-socket NDJSON shape. The pair-
 *   on-connect handshake runs in `./remote-bridge` before any RPC
 *   is allowed.
 */
export async function invoke<K extends TauriCommand>(
  ...args: TauriCommandArgs[K] extends void ? [command: K] : [command: K, args: TauriCommandArgs[K]]
): Promise<TauriCommandResult[K]> {
  const [command, payload] = args

  try {
    if (isRemoteHost()) {
      return (await remoteInvoke(command, payload as Record<string, unknown> | undefined)) as TauriCommandResult[K]
    }

    return await tauriInvoke<TauriCommandResult[K]>(command, payload as Record<string, unknown> | undefined)
  } catch(err) {
    log.error('invoke failed', { command, args: payload }, err)
    throw err
  }
}

export async function listen<K extends TauriEvent>(event: K, cb: EventCallback<TauriEventPayload[K]>): Promise<UnlistenFn> {
  if (isRemoteHost()) {
    return remoteListen(event, cb as EventCallback<unknown>)
  }

  return tauriListen<TauriEventPayload[K]>(event, cb)
}

export type { EventCallback, UnlistenFn }
