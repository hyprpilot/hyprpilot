/**
 * Tool-call → `ToolChipItem` registry. Port of the Python
 * `ToolFormatters` (see `~/.dotfiles/wayland/.config/wayland/scripts/
 * lib/tools.py`) adapted to the overlay's chip shape — the Python
 * emits markdown; here we emit `{ label, arg, detail, stat, kind,
 * state }` so the big-row / small-pill primitives can render without
 * re-parsing.
 *
 * ## Architecture — layered registry
 *
 * The base registry covers the Claude-shaped tool set every ACP agent
 * overlaps on (`read`, `write`, `bash`, …). Per-adapter registries
 * extend the base via `extendRegistry(base, patch)` — overriding
 * individual formatters, aliases, or short-headers without touching
 * the shared path. `resolveRegistry(provider)` picks the right
 * registry for an active adapter; `formatToolCall(call, provider?)`
 * is the public facade.
 *
 * Today every per-adapter registry ships empty (all three vendors
 * inherit the base). Real overrides land per issue as we discover
 * divergence — e.g. opencode's `diagnostics` envelope, codex-specific
 * `$bash_id` semantics, claude-code's MCP tool catalog.
 *
 * ## Dispatch axes (shared across registries)
 *
 *   - name normalisation (`Read`/`read`/`read-file` → `read`) so
 *     claude-agent-acp (PascalCase) and opencode (lowercase) both
 *     land on the same formatter;
 *   - key normalisation inside each formatter (`filePath` /
 *     `file_path`) — `normaliseArgs` is called once on entry; the
 *     per-formatter `pickString` / `pickNumber` / `pickList` helpers
 *     assume already-normalised keys.
 *
 * MCP fallback (`mcp__<server>__<tool>`) extracts server + leaf so
 * the chip shows the server as label and the tool name as arg — no
 * formatter has to know every MCP tool that exists.
 */
import { ToolKind, ToolState } from '@components'
import type { ToolChipItem } from '@components'

import type { ToolCallView } from '../composables/useTools'

/** Short verb-glyph rendered as the chip's leading label. */
const BASE_SHORT_HEADERS = {
  bash: '$',
  bash_output: '$·',
  kill_shell: '$✕',
  read: 'R',
  write: '⇲',
  edit: '✎',
  multi_edit: '✎',
  grep: '/',
  glob: '△',
  web_fetch: '⇩',
  web_search: '?',
  task: '›_',
  todo_write: '☰',
  notebook_edit: '✎nb',
  plan_enter: '◎',
  plan_exit: '◎·',
  terminal: '›_'
} as const

type BaseFormatterKey = keyof typeof BASE_SHORT_HEADERS

const BASE_ALIASES: Record<string, BaseFormatterKey> = {
  bashoutput: 'bash_output',
  killshell: 'kill_shell',
  multiedit: 'multi_edit',
  patch: 'edit',
  webfetch: 'web_fetch',
  websearch: 'web_search',
  agent: 'task',
  todowrite: 'todo_write',
  todo: 'todo_write',
  notebookedit: 'notebook_edit',
  enterplanmode: 'plan_enter',
  exitplanmode: 'plan_exit'
}

type Args = Record<string, unknown>

export interface FormatterContext {
  /** Normalised canonical tool name (lowercase, `_` stripped via normalisation + aliases resolved). */
  name: string
  /** Raw tool name as emitted by the agent, before normalisation. */
  rawName: string
  /** Argument map with every key normalised via `normaliseKey`. */
  args: Args
  state: ToolState
}

export type ToolFormatter = (ctx: FormatterContext) => ToolChipItem

export interface ToolFormatterRegistry {
  /** Canonical name → short verb-glyph shown as the chip label. */
  shortHeaders: Record<string, string>
  /** Raw-or-canonical name → canonical name to look up in `formatters`. */
  aliases: Record<string, string>
  /** Canonical name → chip builder. */
  formatters: Record<string, ToolFormatter>
  /**
   * Fallback used when no formatter matches. The base registry sets
   * this to `formatDefaultFallback` which handles the MCP + unknown
   * paths; adapters should rarely override it.
   */
  fallback: ToolFormatter
}

function normaliseName(raw: string | undefined): string {
  if (!raw) {
    return ''
  }

  return raw.trim().toLowerCase().replace(/-/g, '_')
}

function normaliseKey(key: string): string {
  return key.toLowerCase().replace(/_/g, '')
}

function normaliseArgs(raw: Args | undefined): Args {
  if (!raw) {
    return {}
  }
  const out: Args = {}
  for (const [k, v] of Object.entries(raw)) {
    out[normaliseKey(k)] = v
  }

  return out
}

function pickString(args: Args, ...keys: string[]): string {
  for (const key of keys) {
    const v = args[key]
    if (typeof v === 'string' && v.length > 0) {
      return v
    }
  }

  return ''
}

function pickNumber(args: Args, ...keys: string[]): number | undefined {
  for (const key of keys) {
    const v = args[key]
    if (typeof v === 'number' && Number.isFinite(v)) {
      return v
    }
  }

  return undefined
}

function pickList(args: Args, ...keys: string[]): unknown[] | undefined {
  for (const key of keys) {
    const v = args[key]
    if (Array.isArray(v)) {
      return v
    }
  }

  return undefined
}

function pickStringList(args: Args, ...keys: string[]): string[] {
  const list = pickList(args, ...keys)
  if (!list) {
    return []
  }

  return list.filter((v): v is string => typeof v === 'string' && v.length > 0)
}

function mapToolStatus(raw: string | undefined): ToolState {
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

/** Trim a long path to its last two segments so the chip stays narrow. */
function shortPath(path: string): string {
  if (!path) {
    return ''
  }
  const parts = path.split('/').filter(Boolean)
  if (parts.length <= 2) {
    return path
  }

  return `.../${parts.slice(-2).join('/')}`
}

function titleCase(raw: string): string {
  if (!raw) {
    return ''
  }

  return raw.charAt(0).toUpperCase() + raw.slice(1).toLowerCase()
}

// ── Base formatters ─────────────────────────────────────────────

function formatRead({ args, state }: FormatterContext): ToolChipItem {
  const path = pickString(args, 'filepath', 'path')
  const offset = pickNumber(args, 'offset')
  const limit = pickNumber(args, 'limit')
  let detail: string | undefined
  if (offset !== undefined && limit !== undefined) {
    detail = `lines ${offset}..${offset + limit}`
  } else if (offset !== undefined) {
    detail = `from line ${offset}`
  }

  return {
    label: BASE_SHORT_HEADERS.read,
    arg: shortPath(path),
    detail,
    state,
    kind: ToolKind.Read
  }
}

function formatWrite({ args, state }: FormatterContext): ToolChipItem {
  const path = pickString(args, 'filepath', 'path')
  const content = pickString(args, 'content', 'newstring')
  const stat = content.length > 0 ? `${content.length} chars` : undefined

  return {
    label: BASE_SHORT_HEADERS.write,
    arg: shortPath(path),
    stat,
    state,
    kind: ToolKind.Write
  }
}

function formatEdit({ name, args, state }: FormatterContext): ToolChipItem {
  const path = pickString(args, 'filepath', 'path')
  const replaceAll = Boolean(args.replaceall)
  const edits = pickList(args, 'edits')
  let detail: string | undefined
  let stat: string | undefined
  if (name === 'multi_edit' && edits) {
    stat = `${edits.length} edits`
  } else if (replaceAll) {
    detail = 'replace all'
  }

  return {
    label: BASE_SHORT_HEADERS.edit,
    arg: shortPath(path),
    detail,
    stat,
    state,
    kind: ToolKind.Write
  }
}

function formatBash({ args, state }: FormatterContext): ToolChipItem {
  const command = pickString(args, 'command')
  const description = pickString(args, 'description')
  const background = Boolean(args.runinbackground)
  let detail: string | undefined
  if (description && background) {
    detail = `${description} (background)`
  } else if (description) {
    detail = description
  } else if (background) {
    detail = 'background'
  }

  return {
    label: BASE_SHORT_HEADERS.bash,
    arg: command,
    detail,
    state,
    kind: ToolKind.Bash
  }
}

function formatBashOutput({ args, state }: FormatterContext): ToolChipItem {
  const shellId = pickString(args, 'bashid', 'shellid')
  const filter = pickString(args, 'filter')

  return {
    label: BASE_SHORT_HEADERS.bash_output,
    arg: shellId,
    detail: filter ? `filter ${filter}` : undefined,
    state,
    kind: ToolKind.Bash
  }
}

function formatKillShell({ args, state }: FormatterContext): ToolChipItem {
  const shellId = pickString(args, 'shellid', 'bashid')

  return {
    label: BASE_SHORT_HEADERS.kill_shell,
    arg: shellId,
    state,
    kind: ToolKind.Bash
  }
}

function formatGrep({ args, state }: FormatterContext): ToolChipItem {
  const pattern = pickString(args, 'pattern')
  const path = pickString(args, 'path') || '.'
  const glob = pickString(args, 'glob', 'include')
  const type = pickString(args, 'type')
  const mode = pickString(args, 'outputmode')
  const bits: string[] = [`in ${shortPath(path)}`]
  if (glob) {
    bits.push(`glob=${glob}`)
  }
  if (type) {
    bits.push(`type=${type}`)
  }
  if (mode) {
    bits.push(`mode=${mode}`)
  }
  if (args['-i']) {
    bits.push('-i')
  }
  if (args['-n']) {
    bits.push('-n')
  }

  return {
    label: BASE_SHORT_HEADERS.grep,
    arg: pattern,
    detail: bits.join(' '),
    state,
    kind: ToolKind.Search
  }
}

function formatGlob({ args, state }: FormatterContext): ToolChipItem {
  const pattern = pickString(args, 'pattern')
  const path = pickString(args, 'path')

  return {
    label: BASE_SHORT_HEADERS.glob,
    arg: pattern,
    detail: path ? `in ${shortPath(path)}` : undefined,
    state,
    kind: ToolKind.Search
  }
}

function formatTask({ args, state }: FormatterContext): ToolChipItem {
  const subagent = pickString(args, 'subagenttype') || 'agent'
  const description = pickString(args, 'description')
  const prompt = pickString(args, 'prompt')
  const truncated = prompt.length > 200 ? `${prompt.slice(0, 199)}…` : prompt
  const bits = [description, truncated].filter((s) => s.length > 0)

  return {
    label: BASE_SHORT_HEADERS.task,
    arg: subagent,
    detail: bits.length > 0 ? bits.join(' — ') : undefined,
    state,
    kind: ToolKind.Agent
  }
}

function formatWebFetch({ args, state }: FormatterContext): ToolChipItem {
  const url = pickString(args, 'url', 'uri')
  const prompt = pickString(args, 'prompt')

  return {
    label: BASE_SHORT_HEADERS.web_fetch,
    arg: url,
    detail: prompt || undefined,
    state,
    kind: ToolKind.Acp
  }
}

function formatWebSearch({ args, state }: FormatterContext): ToolChipItem {
  const query = pickString(args, 'query')
  const allowed = pickStringList(args, 'alloweddomains')
  const blocked = pickStringList(args, 'blockeddomains')
  const bits: string[] = []
  if (allowed.length > 0) {
    bits.push(`allowed: ${allowed.join(', ')}`)
  }
  if (blocked.length > 0) {
    bits.push(`blocked: ${blocked.join(', ')}`)
  }

  return {
    label: BASE_SHORT_HEADERS.web_search,
    arg: query,
    detail: bits.length > 0 ? bits.join(' · ') : undefined,
    state,
    kind: ToolKind.Search
  }
}

function formatTerminal({ args, state }: FormatterContext): ToolChipItem {
  const terminalId = pickString(args, 'terminalid', 'id')
  const command = pickString(args, 'command')

  return {
    label: BASE_SHORT_HEADERS.terminal,
    arg: terminalId || command,
    detail: terminalId && command ? command : undefined,
    state,
    kind: ToolKind.Terminal
  }
}

function formatNotebookEdit({ args, state }: FormatterContext): ToolChipItem {
  const path = pickString(args, 'notebookpath', 'filepath')
  const cellId = pickString(args, 'cellid')
  const mode = pickString(args, 'editmode')
  const bits: string[] = []
  if (cellId) {
    bits.push(`cell=${cellId}`)
  }
  if (mode) {
    bits.push(`mode=${mode}`)
  }

  return {
    label: BASE_SHORT_HEADERS.notebook_edit,
    arg: shortPath(path),
    detail: bits.length > 0 ? bits.join(' ') : undefined,
    state,
    kind: ToolKind.Write
  }
}

function formatTodoWrite({ args, state }: FormatterContext): ToolChipItem {
  const todos = pickList(args, 'todos') ?? []
  const count = todos.length
  const counts: Record<string, number> = {}
  for (const entry of todos) {
    if (entry && typeof entry === 'object') {
      const status = (entry as Record<string, unknown>).status
      if (typeof status === 'string') {
        counts[status] = (counts[status] ?? 0) + 1
      }
    }
  }
  const bits = Object.entries(counts).map(([k, v]) => `${k}:${v}`)

  return {
    label: BASE_SHORT_HEADERS.todo_write,
    arg: `${count} ${count === 1 ? 'item' : 'items'}`,
    detail: bits.length > 0 ? bits.join(' ') : undefined,
    state,
    kind: ToolKind.Think
  }
}

function formatPlanEnter({ args, state }: FormatterContext): ToolChipItem {
  const plan = pickString(args, 'plan')
  const truncated = plan.length > 160 ? `${plan.slice(0, 159)}…` : plan

  return {
    label: BASE_SHORT_HEADERS.plan_enter,
    arg: truncated || undefined,
    state,
    kind: ToolKind.Think
  }
}

function formatPlanExit({ args, state }: FormatterContext): ToolChipItem {
  const plan = pickString(args, 'plan')
  const truncated = plan.length > 160 ? `${plan.slice(0, 159)}…` : plan

  return {
    label: BASE_SHORT_HEADERS.plan_exit,
    arg: truncated || undefined,
    state,
    kind: ToolKind.Think
  }
}

function parseMcpName(name: string): { server: string, tool: string } | undefined {
  if (!name.startsWith('mcp__')) {
    return undefined
  }
  const parts = name.split('__')
  if (parts.length < 3) {
    return undefined
  }

  return { server: parts[1] ?? '', tool: parts.slice(2).join('__') }
}

function summariseArgs(args: Args): string {
  const first = Object.entries(args).find(([, v]) => typeof v === 'string' && v.length > 0)
  if (!first) {
    return ''
  }
  const [, v] = first

  return typeof v === 'string' ? v : ''
}

/**
 * Last-resort formatter used when the registry has no specific
 * formatter for `name`. Handles MCP-style names (`mcp__server__tool`)
 * by extracting the server as label and the tool leaf as arg; falls
 * back to the fallback short-header logic for everything else.
 */
function formatDefaultFallback({ name, rawName, args, state }: FormatterContext): ToolChipItem {
  const mcp = parseMcpName(name)
  if (mcp) {
    const summary = summariseArgs(args)
    const toolLabel = mcp.tool.replace(/_/g, ' ')

    return {
      label: mcp.server,
      arg: summary ? `${toolLabel}(${summary})` : toolLabel,
      state,
      kind: ToolKind.Acp
    }
  }

  return {
    label: fallbackShortHeader(name, rawName),
    arg: summariseArgs(args) || rawName,
    state,
    kind: ToolKind.Acp
  }
}

function fallbackShortHeader(canonName: string, rawName: string): string {
  const mcp = parseMcpName(canonName)
  if (mcp) {
    const words = mcp.tool.replace(/_/g, ' ').trim()

    return words.length > 0 ? titleCase(words) : '·'
  }
  if (canonName.length === 0) {
    return rawName || '·'
  }

  return titleCase(canonName.replace(/_/g, ' '))
}

// ── Base registry ────────────────────────────────────────────────

const BASE_FORMATTERS: Record<BaseFormatterKey, ToolFormatter> = {
  read: formatRead,
  write: formatWrite,
  edit: formatEdit,
  multi_edit: formatEdit,
  bash: formatBash,
  bash_output: formatBashOutput,
  kill_shell: formatKillShell,
  grep: formatGrep,
  glob: formatGlob,
  task: formatTask,
  web_fetch: formatWebFetch,
  web_search: formatWebSearch,
  terminal: formatTerminal,
  notebook_edit: formatNotebookEdit,
  todo_write: formatTodoWrite,
  plan_enter: formatPlanEnter,
  plan_exit: formatPlanExit
}

// `BASE_FORMATTERS: Record<BaseFormatterKey, ToolFormatter>` pins every
// key declared in `BASE_SHORT_HEADERS` to a formatter at compile time —
// the `todo_write` / `plan_enter` / `plan_exit` silent-no-op case the
// pre-rework code shipped would be a TS error here now.

export const baseRegistry: ToolFormatterRegistry = {
  shortHeaders: BASE_SHORT_HEADERS,
  aliases: BASE_ALIASES,
  formatters: BASE_FORMATTERS,
  fallback: formatDefaultFallback
}

/**
 * Return a new registry composed of `base` with `patch` layered on
 * top. Undefined fields in `patch` inherit straight from `base`;
 * defined fields spread so adapter overrides replace individual
 * entries without clobbering the rest of the map.
 */
export function extendRegistry(
  base: ToolFormatterRegistry,
  patch: {
    shortHeaders?: Record<string, string>
    aliases?: Record<string, string>
    formatters?: Record<string, ToolFormatter>
    fallback?: ToolFormatter
  }
): ToolFormatterRegistry {
  return {
    shortHeaders: { ...base.shortHeaders, ...(patch.shortHeaders ?? {}) },
    aliases: { ...base.aliases, ...(patch.aliases ?? {}) },
    formatters: { ...base.formatters, ...(patch.formatters ?? {}) },
    fallback: patch.fallback ?? base.fallback
  }
}

/**
 * Per-adapter registries. Today every vendor inherits the base
 * untouched; per-adapter divergence (opencode `diagnostics` leftover
 * surfacing, codex `$bash_id` semantics, claude-code MCP tools)
 * lands here as it's discovered.
 */
export const registries: Record<string, ToolFormatterRegistry> = {
  'acp-claude-code': extendRegistry(baseRegistry, {}),
  'acp-codex': extendRegistry(baseRegistry, {}),
  'acp-opencode': extendRegistry(baseRegistry, {})
}

/**
 * Resolve the registry for a given provider. Unknown providers fall
 * back to the base registry — safe because the base covers every
 * tool ACP agents overlap on.
 */
export function resolveRegistry(provider?: string): ToolFormatterRegistry {
  if (provider && Object.prototype.hasOwnProperty.call(registries, provider)) {
    return registries[provider]!
  }

  return baseRegistry
}

function resolveCanonical(registry: ToolFormatterRegistry, rawName: string): string {
  const normalised = normaliseName(rawName)

  return registry.aliases[normalised] ?? normalised
}

/**
 * Short verb-glyph for a tool name. MCP tools fall back to the leaf
 * name title-cased; unknown built-ins title-case the full canonical
 * leaf (`somecommand` → `Somecommand`); empty input to `·`. Matches
 * the Python reference.
 */
export function shortHeader(toolName: string, provider?: string): string {
  const registry = resolveRegistry(provider)
  const canon = resolveCanonical(registry, toolName)
  const hit = registry.shortHeaders[canon]
  if (hit) {
    return hit
  }

  return fallbackShortHeader(canon, toolName)
}

/**
 * Public facade: turn a `ToolCallView` into a `ToolChipItem`. Routes
 * through the registry resolved for `provider` (base when omitted /
 * unknown).
 */
export function formatToolCall(call: ToolCallView, provider?: string): ToolChipItem {
  const registry = resolveRegistry(provider)
  const rawName = call.title ?? ''
  const canon = resolveCanonical(registry, rawName)
  const args = normaliseArgs(call.rawInput)
  const state = mapToolStatus(call.status)
  const ctx: FormatterContext = { name: canon, rawName, args, state }

  const formatter = registry.formatters[canon]
  if (formatter) {
    return formatter(ctx)
  }

  return registry.fallback(ctx)
}

/**
 * Expanded-row markdown body.
 *
 * @todo Wire up when the row-big surface grows an expanded panel.
 *       The Python reference emits markdown via `format_bash` etc.
 *       and then appends a trailing `_leftover_json_block` for any
 *       argument the formatter didn't pop — so agent-specific extras
 *       (opencode's `diagnostics`, `_meta` envelopes, tool-specific
 *       fields the formatter doesn't know about yet) stay visible.
 *       When `formatToolBody` lands it needs that pop-and-leftover
 *       machinery (today `FormatterContext.args` is read-only), or
 *       agent-tacked-on fields disappear silently. Track via a
 *       follow-up K-xxx issue.
 */
export function formatToolBody(_call: ToolCallView, _provider?: string): string {
  if (import.meta.env.DEV) {
    throw new Error('formatToolBody: expanded-row markdown not implemented yet (follow-up issue)')
  }

  return ''
}
