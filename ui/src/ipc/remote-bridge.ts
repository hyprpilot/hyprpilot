/**
 * Remote-WS transport for the IPC bridge. When the SPA loads in a
 * browser (no `window.__TAURI_INTERNALS__`), `bridge.ts` routes
 * `invoke()` / `listen()` here instead of `@tauri-apps/api/core`.
 *
 * Wire shape mirrors the unix-socket NDJSON dispatcher:
 * - **Outbound RPC**: `{ "jsonrpc": "2.0", "id": <uuid>, "method": "...", "params": {...} }`
 * - **Inbound RPC reply**: `{ "jsonrpc": "2.0", "id": <uuid>, "result": ... | "error": {...} }`
 * - **Inbound event push**: `{ "type": "event", "name": "acp:transcript", "payload": ... }`
 *   (renamed envelope; events don't ride the JSON-RPC envelope.)
 * - **Inbound pair frames**: `{ "type": "pending" | "authenticated" | "rejected" }`
 *
 * The Tauri command boundary is wider than the JSON-RPC surface — many
 * UI calls go through Tauri-only commands (`get_theme`, `instance_meta`,
 * `models_set`, etc.) that have no JSON-RPC mirror today. To keep one
 * call signature for the UI, we route every Tauri command name
 * straight to a `tauri/<command>` JSON-RPC method, and the daemon's
 * remote dispatcher proxies them through to the Tauri command handler.
 *
 * Pair handshake: connection opens, daemon sends `pending` frame
 * with the BIP39 code; the captain confirms on the desktop overlay;
 * daemon sends `authenticated`; we then drain queued RPC calls.
 */

import type { EventCallback, UnlistenFn } from '@tauri-apps/api/event'

let socket: WebSocket | undefined
let connectPromise: Promise<WebSocket> | undefined
let authenticated = false
let pendingFrame: PendingFrame | undefined

interface PendingFrame {
  pendingId: string
  code: string
  qrPayload: string
  expiresInSeconds: number
}

interface RpcResolver {
  resolve: (value: unknown) => void
  reject: (reason: unknown) => void
}

const inflight = new Map<string, RpcResolver>()
const eventListeners = new Map<string, Set<EventCallback<unknown>>>()
const pairListeners: Set<(frame: PendingFrame | undefined) => void> = new Set()

/**
 * Probe at boot — Tauri injects `window.__TAURI_INTERNALS__` BEFORE
 * the SPA's bootstrap runs. When it's missing, the SPA is being
 * served by the daemon's HTTPS bridge to a browser.
 */
export function isRemoteHost(): boolean {
  return typeof window !== 'undefined' && !('__TAURI_INTERNALS__' in window)
}

function buildWsUrl(): string {
  // In production the SPA is served by the same axum server that
  // hosts the WS endpoint, so `wss://<location.host>/ws` reaches
  // the same daemon under the same TLS cert.
  const proto = window.location.protocol === 'https:' ? 'wss:' : 'ws:'

  return `${proto}//${window.location.host}/ws`
}

async function connect(): Promise<WebSocket> {
  if (socket && socket.readyState === WebSocket.OPEN) {
    return socket
  }

  if (connectPromise) {
    return connectPromise
  }
  connectPromise = new Promise<WebSocket>((resolve, reject) => {
    const ws = new WebSocket(buildWsUrl())

    ws.addEventListener('open', () => {
      socket = ws
      resolve(ws)
    })
    ws.addEventListener('error', (ev) => {
      connectPromise = undefined
      reject(ev)
    })
    ws.addEventListener('close', () => {
      socket = undefined
      authenticated = false
      pendingFrame = undefined
      notifyPairListeners()
      // Reject every in-flight RPC so callers see the disconnect.
      const queued = [...inflight.values()]

      inflight.clear()

      for (const r of queued) {
        r.reject(new Error('remote bridge: WS closed'))
      }
      connectPromise = undefined
    })
    ws.addEventListener('message', onMessage)
  })

  return connectPromise
}

function onMessage(ev: MessageEvent): void {
  const data = typeof ev.data === 'string' ? ev.data : ''

  if (!data) {
    return
  }
  let parsed: unknown

  try {
    parsed = JSON.parse(data)
  } catch {
    return
  }

  if (!parsed || typeof parsed !== 'object') {
    return
  }
  const msg = parsed as Record<string, unknown>

  // Pair-handshake frames carry a `type` discriminator.
  if (typeof msg.type === 'string') {
    handleFrameByType(msg)

    return
  }

  // JSON-RPC reply (id present).
  if (msg.id !== undefined && msg.jsonrpc === '2.0') {
    const id = String(msg.id)
    const resolver = inflight.get(id)

    if (!resolver) {
      return
    }
    inflight.delete(id)

    if ('error' in msg && msg.error) {
      const err = msg.error as { message?: string; code?: number }

      resolver.reject(new Error(err.message ?? 'remote rpc error'))
    } else {
      resolver.resolve(msg.result)
    }
  }
}

function handleFrameByType(msg: Record<string, unknown>): void {
  const type = String(msg.type)

  switch (type) {
    case 'pending':
      pendingFrame = {
        pendingId: String(msg.pendingId),
        code: String(msg.code),
        qrPayload: String(msg.qrPayload),
        expiresInSeconds: Number(msg.expiresInSeconds ?? 60)
      }
      notifyPairListeners()

      return
    case 'authenticated':
      authenticated = true
      pendingFrame = undefined
      notifyPairListeners()

      return
    case 'rejected':
      authenticated = false
      pendingFrame = undefined
      notifyPairListeners()

      if (socket) {
        socket.close()
      }

      return
    case 'event': {
      const name = String(msg.name)
      const set = eventListeners.get(name)

      if (!set) {
        return
      }

      for (const cb of set) {
        try {
          cb({
            event: name, payload: msg.payload, id: 0
          } as Parameters<EventCallback<unknown>>[0])
        } catch(err) {
        // Listener errors must not derail the multiplexer.
        // eslint-disable-next-line no-console
          console.warn('remote bridge: event listener threw', err)
        }
      }

      return
    }
    default:
    // Unknown frame types are tolerated for forward-compat; future
    // daemon versions may add new envelope types.
      return
  }
}

function notifyPairListeners(): void {
  for (const cb of pairListeners) {
    cb(pendingFrame)
  }
}

async function ensureAuthenticated(): Promise<WebSocket> {
  const ws = await connect()

  if (authenticated) {
    return ws
  }
  // Block until authenticated — captain confirms on desktop.
  await new Promise<void>((resolve, reject) => {
    const onClose = (): void => {
      ws.removeEventListener('close', onClose)
      pairListeners.delete(handler)
      reject(new Error('remote bridge: WS closed before pair confirm'))
    }
    const handler = (frame: PendingFrame | undefined): void => {
      if (authenticated) {
        ws.removeEventListener('close', onClose)
        pairListeners.delete(handler)
        resolve()
      } else if (!frame) {
        // pendingFrame cleared without authentication → rejected
        ws.removeEventListener('close', onClose)
        pairListeners.delete(handler)
        reject(new Error('remote bridge: pair rejected'))
      }
    }

    ws.addEventListener('close', onClose)
    pairListeners.add(handler)
  })

  return ws
}

export async function remoteInvoke(command: string, args?: Record<string, unknown>): Promise<unknown> {
  const ws = await ensureAuthenticated()
  const id = crypto.randomUUID()
  const frame = JSON.stringify({
    jsonrpc: '2.0',
    id,
    method: `tauri/${command}`,
    params: args ?? {}
  })

  return new Promise<unknown>((resolve, reject) => {
    inflight.set(id, { resolve, reject })

    try {
      ws.send(frame)
    } catch(err) {
      inflight.delete(id)
      reject(err)
    }
  })
}

export async function remoteListen(event: string, cb: EventCallback<unknown>): Promise<UnlistenFn> {
  await connect()
  let set = eventListeners.get(event)

  if (!set) {
    set = new Set()
    eventListeners.set(event, set)
  }
  set.add(cb)

  return () => {
    set?.delete(cb)

    if (set?.size === 0) {
      eventListeners.delete(event)
    }
  }
}

/**
 * Public surface for the pair-flow UI. The remote landing page
 * subscribes to receive `pending` frames so it can render the QR +
 * code for the captain to read across to the desktop.
 */
export interface PairView {
  pending?: PendingFrame
  authenticated: boolean
}

export function subscribePair(cb: (view: PairView) => void): () => void {
  const handler = (): void => cb({ pending: pendingFrame, authenticated })

  pairListeners.add(handler)
  // Fire immediately so the consumer sees the current state.
  handler()

  return () => {
    pairListeners.delete(handler)
  }
}

export function getRemotePairView(): PairView {
  return { pending: pendingFrame, authenticated }
}

export function ensureRemoteConnection(): Promise<void> {
  return connect().then(() => undefined)
}
