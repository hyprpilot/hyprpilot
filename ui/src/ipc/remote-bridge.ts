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
let lastConfirmRejection: string | undefined
/**
 * Survives the WS close that follows a `rejected` / expiry frame so
 * the mobile pair screen can render a "the desktop rejected this
 * pair request" banner instead of bouncing back to a generic
 * "connecting…" loader. Cleared when the user retries (a fresh
 * `pending` frame lands on a new connection).
 */
let terminalReason: string | undefined

interface PendingFrame {
  pendingId: string
  /** Code shown on this device — also the QR payload here. */
  deviceCode: string
  /** Code shown on the desktop — what we must scan / receive to authenticate from this side. */
  desktopCode: string
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
      lastConfirmRejection = undefined

      // Preserve `terminalReason` across the close — that's what tells
      // the mobile UI "this pair was rejected" vs "we just lost the
      // connection". A connection drop with no preceding `rejected`
      // frame fills it with a generic message.
      if (terminalReason === undefined) {
        terminalReason = 'connection lost'
      }
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
        deviceCode: String(msg.deviceCode),
        desktopCode: String(msg.desktopCode),
        expiresInSeconds: Number(msg.expiresInSeconds ?? 60)
      }
      // Fresh pending = fresh connection = retry succeeded; clear
      // any leftover terminal/rejection text from a prior attempt.
      lastConfirmRejection = undefined
      terminalReason = undefined
      notifyPairListeners()

      return
    case 'authenticated':
      authenticated = true
      pendingFrame = undefined
      lastConfirmRejection = undefined
      terminalReason = undefined
      notifyPairListeners()

      return
    case 'rejected':
      authenticated = false
      pendingFrame = undefined
      lastConfirmRejection = undefined
      // Preserved through the close handler so the mobile UI can
      // show a dedicated "rejected" screen instead of falling back
      // to the generic "connecting…" loader.
      terminalReason = typeof msg.reason === 'string' && msg.reason.length > 0 ? msg.reason : 'pair request rejected'
      notifyPairListeners()

      if (socket) {
        socket.close()
      }

      return
    case 'confirm-rejected':
      // Daemon refused a `{type:"confirm"}` frame the SPA pushed —
      // most often because the captain scanned the wrong QR /
      // desktop's QR was stale. Pending state stays alive (until
      // expiry / attempt cap), so the user can retry the scan.
      lastConfirmRejection = typeof msg.reason === 'string' ? msg.reason : 'pair code did not match'
      notifyPairListeners()

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
  /**
   * Last `confirm-rejected` reason received from the daemon. Set when
   * a `confirmFromBrowser()` push gets refused (mostly: scan decoded
   * the wrong QR). Cleared on the next pending/auth/rejected frame so
   * the mobile UI can flash a one-shot error pill.
   */
  lastConfirmRejection?: string
  /**
   * Terminal reason for a closed WS — populated when the daemon sent
   * `{type:"rejected"}` (captain hit reject / pair expired / 3 wrong
   * codes) or when the connection dropped without an explicit
   * rejection. Survives the `close` event so the mobile UI can render
   * a dedicated "rejected" view rather than the generic loader.
   * Cleared on the next `pending` frame (retry succeeded).
   */
  terminalReason?: string
}

function buildPairView(): PairView {
  return {
    pending: pendingFrame, authenticated, lastConfirmRejection, terminalReason
  }
}

export function subscribePair(cb: (view: PairView) => void): () => void {
  const handler = (): void => cb(buildPairView())

  pairListeners.add(handler)
  // Fire immediately so the consumer sees the current state.
  handler()

  return () => {
    pairListeners.delete(handler)
  }
}

export function getRemotePairView(): PairView {
  return buildPairView()
}

/**
 * Push a `{type:"confirm", code}` frame back over the pending WS.
 * The daemon checks the code against the one it minted for *this*
 * connection — match → fires the same oneshot a desktop confirm
 * fires, WS upgrades to authenticated. Used by the mobile pair
 * screen's webcam-scan path: phone reads the desktop's QR and
 * pushes the decoded code back.
 *
 * Throws if the WS isn't open (caller already saw a `pending` frame
 * so the connection should be live; defensive throw covers races).
 */
export function confirmFromBrowser(code: string): void {
  if (!socket || socket.readyState !== WebSocket.OPEN) {
    throw new Error('remote bridge: WS not connected')
  }
  // Clear any stale rejection — the user is retrying.
  lastConfirmRejection = undefined
  notifyPairListeners()
  socket.send(JSON.stringify({ type: 'confirm', code: code.trim() }))
}

/**
 * Force a fresh pair attempt after a `terminalReason` landed. Reloads
 * the page so the SPA re-runs its boot from a clean slate — re-mints
 * the WS, picks up a fresh pending code, etc. Page reload is the
 * coarsest correct retry; a finer-grained reset (just the WS, keep
 * the SPA) would have to manage all the in-flight RPC promises that
 * `close()` rejected on the previous run, and the boot path is cheap
 * enough that one reload is the simpler answer.
 */
export function retryRemotePair(): void {
  if (typeof window !== 'undefined') {
    window.location.reload()
  }
}

export function ensureRemoteConnection(): Promise<void> {
  return connect().then(() => undefined)
}
