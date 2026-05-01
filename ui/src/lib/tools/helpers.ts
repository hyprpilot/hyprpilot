import { ToolState } from '@components'

import type { Args } from '@interfaces/ui/tools'

export function normaliseName(raw: string | undefined): string {
  if (!raw) {
    return ''
  }

  return raw.trim().toLowerCase().replace(/-/g, '_')
}

export function normaliseKey(key: string): string {
  return key.toLowerCase().replace(/_/g, '')
}

export function normaliseArgs(raw: Args | undefined): Args {
  if (!raw) {
    return {}
  }
  const out: Args = {}
  for (const [k, v] of Object.entries(raw)) {
    out[normaliseKey(k)] = v
  }

  return out
}

export function pickString(args: Args, ...keys: string[]): string {
  for (const key of keys) {
    const v = args[key]
    if (typeof v === 'string' && v.length > 0) {
      return v
    }
  }

  return ''
}

export function pickNumber(args: Args, ...keys: string[]): number | undefined {
  for (const key of keys) {
    const v = args[key]
    if (typeof v === 'number' && Number.isFinite(v)) {
      return v
    }
  }

  return undefined
}

export function pickList(args: Args, ...keys: string[]): unknown[] | undefined {
  for (const key of keys) {
    const v = args[key]
    if (Array.isArray(v)) {
      return v
    }
  }

  return undefined
}

export function pickStringList(args: Args, ...keys: string[]): string[] {
  const list = pickList(args, ...keys)
  if (!list) {
    return []
  }

  return list.filter((v): v is string => typeof v === 'string' && v.length > 0)
}

export function mapToolStatus(raw: string | undefined): ToolState {
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
export function shortPath(path: string): string {
  if (!path) {
    return ''
  }
  const parts = path.split('/').filter(Boolean)
  if (parts.length <= 2) {
    return path
  }

  return `.../${parts.slice(-2).join('/')}`
}

export function titleCase(raw: string): string {
  if (!raw) {
    return ''
  }

  return raw.charAt(0).toUpperCase() + raw.slice(1).toLowerCase()
}

export function parseMcpName(name: string): { server: string; tool: string } | undefined {
  if (!name.startsWith('mcp__')) {
    return undefined
  }
  const parts = name.split('__')
  if (parts.length < 3) {
    return undefined
  }

  return { server: parts[1] ?? '', tool: parts.slice(2).join('__') }
}

export function summariseArgs(args: Args): string {
  const first = Object.entries(args).find(([, v]) => typeof v === 'string' && v.length > 0)
  if (!first) {
    return ''
  }
  const [, v] = first

  return typeof v === 'string' ? v : ''
}
