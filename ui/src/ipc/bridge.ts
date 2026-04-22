import { invoke as tauriInvoke } from '@tauri-apps/api/core'
import { listen as tauriListen, type EventCallback, type UnlistenFn } from '@tauri-apps/api/event'

export async function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  return tauriInvoke<T>(command, args)
}

export async function listen<T>(event: string, cb: EventCallback<T>): Promise<UnlistenFn> {
  return tauriListen<T>(event, cb)
}

export type { EventCallback, UnlistenFn }
