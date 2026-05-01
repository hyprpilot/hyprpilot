import { computed, reactive, type Component, type ComputedRef, type VNode } from 'vue'

import { ToastTone } from '@components'
import { log } from '@lib'

const DEFAULT_DURATION_MS = 3000
const MAX_STACK = 3

/**
 * Toast surface — in-Frame overlay card per wireframe spec.
 *
 * Renderer: `<Frame :toast="...">` (Overlay binds the head of the
 * `entries` ref). Each entry carries a `body` describing what to
 * render INSIDE the toast chrome — the toast frame (tone-stripe,
 * dismiss button, layout) is fixed; everything inside is the
 * consumer's call.
 *
 * `body` accepts three shapes per the compose-not-bag rule
 * (CLAUDE.md):
 *
 *   - **string** — the simplest case. The toast renders the string
 *     in the standard `.toast-message` span. 99% of call sites land
 *     here ("session started", "queue cleared", "restore failed").
 *
 *   - **render function `() => VNode`** — for inline structured
 *     content. Lets a caller compose a label + an action button +
 *     anything else without the composable having to know about
 *     "actions" at all. Example: a cancel-turn toast wires its
 *     own `<button onClick="...">delete</button>` inside the body.
 *
 *   - **`{ component, props? }`** — pass a Vue component reference
 *     and its props. The toast renders that component as the body.
 *     Use when the toast needs richer structure than a one-line
 *     `h(...)` reads cleanly — e.g. a multi-step progress toast.
 *
 * The previous shape — `pushToast(tone, message, { action: { label,
 * run } })` — is gone. Hardcoded "message + action" was the bag
 * smell: every new affordance (icon, badge, dual-action) would
 * have widened the type. Now consumers compose what they need.
 */
export type ToastBody =
  | string
  | (() => VNode)
  | { component: Component; props?: Record<string, unknown> }

export interface ToastEntry {
  id: string
  tone: ToastTone
  body: ToastBody
  createdAt: number
}

export interface ToastOptions {
  /** Auto-dismiss after N ms; `0` disables auto-dismiss. Defaults to 3000. */
  durationMs?: number
}

const entries: ToastEntry[] = reactive([])
let nextId = 0

/**
 * Best-effort string view of a toast body for daemon-side logging.
 * Non-string bodies fall back to the component name (when known)
 * or a generic placeholder — the live in-frame rendering is what
 * the captain sees, the log line is for postmortem context.
 */
function toastBodyToLogString(body: ToastBody): string {
  if (typeof body === 'string') {
    return body
  }
  if (typeof body === 'function') {
    return '<render fn>'
  }
  const name = (body.component as { name?: string }).name ?? '<component>'
  return `<${name}>`
}

/**
 * Map a toast tone onto the log level the captain would expect to
 * find it under in the daemon log. Err goes to error, Warn to warn,
 * everything else (Ok / Info) to info — so a `tail -f hyprpilot.log
 * | grep error` surface their existence without searching the
 * entire stream.
 */
function logToast(tone: ToastTone, body: ToastBody): void {
  const text = toastBodyToLogString(body)
  const fields = { source: 'toast', tone }
  switch (tone) {
    case ToastTone.Err:
      log.error(text, fields)
      break
    case ToastTone.Warn:
      log.warn(text, fields)
      break
    case ToastTone.Ok:
    case ToastTone.Info:
      log.info(text, fields)
      break
  }
}

/**
 * Push a toast onto the FIFO buffer. `body` is `string | (() =>
 * VNode) | { component, props }` per the compose-not-bag rule —
 * the chrome (tone-stripe, dismiss) is fixed; the body is whatever
 * the consumer composes. Mirrored to the daemon log via `log.*` so
 * captain-visible feedback is also captured for postmortems.
 */
export function pushToast(tone: ToastTone, body: ToastBody, options: ToastOptions = {}): string {
  nextId += 1
  const id = `toast-${nextId}`
  const durationMs = options.durationMs ?? DEFAULT_DURATION_MS
  entries.push({ id, tone, body, createdAt: Date.now() })
  while (entries.length > MAX_STACK) {
    entries.shift()
  }
  if (durationMs > 0) {
    window.setTimeout(() => dismissToast(id), durationMs)
  }
  logToast(tone, body)
  return id
}

export function dismissToast(id: string): void {
  const idx = entries.findIndex((e) => e.id === id)
  if (idx >= 0) {
    entries.splice(idx, 1)
  }
}

export function clearToasts(): void {
  entries.length = 0
}

export function useToasts(): {
  entries: ComputedRef<ToastEntry[]>
  push: typeof pushToast
  dismiss: typeof dismissToast
} {
  return {
    entries: computed(() => entries),
    push: pushToast,
    dismiss: dismissToast
  }
}
