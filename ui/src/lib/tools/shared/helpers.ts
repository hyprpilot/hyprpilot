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

/**
 * One ACP `ToolCallContent::Diff` payload. Carries the file path
 * plus old / new text. `oldText` is `null` for new files (per spec);
 * the caller treats null as the empty string when computing a diff.
 */
export interface DiffContentBlock {
  path: string
  oldText: string | null
  newText: string
}

/**
 * Extract every `{ type: "diff", path, oldText, newText }` content
 * block from the wire payload. Returns the empty array when none —
 * caller falls back to plain text rendering.
 *
 * Edit / MultiEdit / Write tool calls all emit Diff blocks (per ACP
 * tool-calls spec); claude-code-acp / codex-acp / opencode-acp ship
 * the same shape. Older clients dropped these on the floor —
 * `textBlocks(...)` only walks `text`-shaped blocks; this helper
 * pulls the Diff variant separately.
 */
export function diffBlocks(content: WireToolCall['content']): DiffContentBlock[] {
  const out: DiffContentBlock[] = []

  for (const block of content ?? []) {
    if (block.type !== 'diff') {
      continue
    }
    const path = typeof block.path === 'string' ? block.path : ''
    const newText = typeof block.newText === 'string' ? block.newText : ''
    const oldText = typeof block.oldText === 'string' ? block.oldText : block.oldText === null ? null : ''

    if (path.length === 0) {
      continue
    }
    out.push({
      path, oldText, newText
    })
  }

  return out
}

/**
 * Path → MIME hint for the rich diff renderer. Returns `undefined`
 * for unrecognised extensions; `richDiffMarkdown` then falls back
 * to plaintext / cheap unified-diff render. Agent-side Diff content
 * blocks don't carry MIME — so file-modifying formatters infer from
 * the path. Read / Fetch get MIME directly from the daemon.
 */
export function inferMimeFromPath(path: string): string | undefined {
  const seg = path.split('/').pop() ?? ''
  const dot = seg.lastIndexOf('.')

  if (dot < 0) {
    return undefined
  }
  const ext = seg.slice(dot + 1).toLowerCase()

  switch (ext) {
    case 'ts':
    case 'tsx':
      return 'application/typescript'

    case 'js':
    case 'jsx':
      return 'application/javascript'

    case 'json':
      return 'application/json'

    case 'md':
      return 'text/markdown'

    case 'yaml':
    case 'yml':
      return 'application/x-yaml'

    case 'toml':
      return 'application/toml'

    case 'rs':
      return 'text/x-rust'

    case 'go':
      return 'text/x-go'

    case 'py':
      return 'text/x-python'

    case 'sh':
      return 'application/x-sh'

    case 'sql':
      return 'application/sql'

    case 'vue':
      return 'text/x-vue'

    case 'html':
      return 'text/html'

    case 'css':
      return 'text/css'

    default:
      return undefined
  }
}
