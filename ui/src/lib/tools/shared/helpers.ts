/**
 * Cross-formatter primitives — state mapping, path trimming, args
 * normalisation, fields projection. Pure helpers; no Vue, no Tauri.
 */

import { ToolState } from '@constants/ui'
import type { Args, ToolField, WireToolCall } from '@interfaces/ui'

/**
 * Lowercase + strip underscores so a formatter's lookup is case-
 * and underscore-insensitive: `pickArgs(normaliseArgs(raw), { ... })`.
 * Produces `filepath` / `bashid` for inputs `file_path` / `bash_id`.
 */
export function normaliseArgs(raw: Args | undefined): Args {
  if (!raw) {
    return {}
  }
  const out: Args = {}

  for (const [k, v] of Object.entries(raw)) {
    out[k.toLowerCase().replace(/_/g, '')] = v
  }

  return out
}

/** Wire status → `ToolState`. */
export function mapState(raw: string | undefined): ToolState {
  switch (raw?.toLowerCase()) {
    case 'completed':
    case 'done':
      return ToolState.Done
    case 'failed':
    case 'error':
      return ToolState.Failed
    case 'awaiting':
    case 'pending':
      return ToolState.Awaiting
    case 'in_progress':
    case 'running':
    default:
      return ToolState.Running
  }
}

/** Trim a long path to its last two segments so the title stays narrow. */
export function shortPath(path: string | undefined): string {
  if (!path) {
    return ''
  }
  const parts = path.split('/').filter(Boolean)

  if (parts.length <= 2) {
    return path
  }

  return `.../${parts.slice(-2).join('/')}`
}

/** First-letter capitalised, rest lower-cased. Sentence-case for chip labels. */
export function sentenceCase(raw: string): string {
  if (!raw) {
    return ''
  }

  return raw.charAt(0).toUpperCase() + raw.slice(1).toLowerCase()
}

/**
 * Project an arg map onto structured `ToolField` rows. Used by the
 * generic MCP formatter and the `Other` fallback. Values render as
 * code blocks in the spec sheet; nested objects fall back to a
 * one-line `JSON.stringify` so the captain still sees the payload.
 *
 * `exclude` lets the caller skip keys that are routed elsewhere on
 * the view (the canonical case: `description` extracts to
 * `view.description` for markdown rendering, so it shouldn't also
 * appear as a field row).
 */
export function argsToFields(args: Args, exclude: readonly string[] = []): ToolField[] | undefined {
  const out: ToolField[] = []
  const skip = new Set(exclude.map((k) => k.toLowerCase().replace(/_/g, '')))

  for (const [k, v] of Object.entries(args)) {
    if (skip.has(k.toLowerCase().replace(/_/g, ''))) {
      continue
    }

    if (v === undefined || v === null) {
      continue
    }
    let value: string

    if (typeof v === 'string') {
      if (v.length === 0) {
        continue
      }
      value = v
    } else if (typeof v === 'number' || typeof v === 'boolean') {
      value = String(v)
    } else {
      try {
        value = JSON.stringify(v)
      } catch {
        continue
      }
    }
    out.push({ label: k, value })
  }

  return out.length > 0 ? out : undefined
}

/** Joined text from every wire content block of `kind: text`. */
export function textBlocks(content: WireToolCall['content']): string {
  const parts: string[] = []

  for (const block of content ?? []) {
    if (typeof block.text === 'string' && block.text.length > 0) {
      parts.push(block.text)
    }
  }

  return parts.join('\n\n')
}
