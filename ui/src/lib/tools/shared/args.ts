/**
 * Schema-driven argument extraction. One helper replaces the prior
 * four (`pickString` / `pickNumber` / `pickList` / `pickStringList`)
 * — declare the expected shape, get a typed object back. Wrong-typed
 * or missing values silently drop to `undefined` so the formatter's
 * optional-chain rendering just works.
 */

import type { Args } from '@interfaces/ui'

export type ArgKind = 'string' | 'number' | 'boolean' | 'list' | 'stringList'

export interface ArgKindToType {
  string: string
  number: number
  boolean: boolean
  list: unknown[]
  stringList: string[]
}

export type ArgSchema = Record<string, ArgKind>

export type InferArgs<S extends ArgSchema> = {
  [K in keyof S]?: ArgKindToType[S[K]]
}

function matchesKind(v: unknown, kind: ArgKind): boolean {
  switch (kind) {
    case 'string':
      return typeof v === 'string' && v.length > 0

    case 'number':
      return typeof v === 'number' && Number.isFinite(v)

    case 'boolean':
      return typeof v === 'boolean'

    case 'list':
      return Array.isArray(v)

    case 'stringList':
      return Array.isArray(v) && v.every((x) => typeof x === 'string' && x.length > 0)
  }
}

/**
 * Walk `schema`, extract each key's value from `args` if it matches
 * the declared kind. The returned object's fields are typed off the
 * schema — `pickArgs(args, { command: 'string' })` returns
 * `{ command?: string }` exactly.
 */
export function pickArgs<S extends ArgSchema>(args: Args, schema: S): InferArgs<S> {
  const out: Record<string, unknown> = {}

  for (const [key, kind] of Object.entries(schema)) {
    const v = args[key]

    if (matchesKind(v, kind)) {
      out[key] = v
    }
  }

  return out as InferArgs<S>
}
