/**
 * Composer autocomplete wire shapes — mirror the Rust types in
 * `src-tauri/src/completion/`. Daemon ranks + truncates; UI renders
 * the items verbatim.
 */

export enum CompletionKind {
  Skill = 'skill',
  Path = 'path',
  Word = 'word',
  Command = 'command'
}

export type CompletionSourceId = 'skills' | 'path' | 'ripgrep' | 'commands'

export interface ReplacementRange {
  start: number
  end: number
}

export interface Replacement {
  range: ReplacementRange
  text: string
}

export interface CompletionItem {
  label: string
  detail?: string
  kind: CompletionKind
  replacement: Replacement
  resolveId?: string
}

export interface CompletionQueryArgs {
  text: string
  cursor: number
  cwd?: string
  manual?: boolean
  instanceId?: string
}

export interface CompletionQueryResponse {
  requestId: string
  sourceId: CompletionSourceId | null
  replacementRange: ReplacementRange | null
  items: CompletionItem[]
}

export interface CompletionResolveArgs {
  resolveId: string
  sourceId: CompletionSourceId
}

export interface CompletionResolveResponse {
  documentation?: string
}

export interface CompletionCancelArgs {
  requestId: string
}

export interface CompletionCancelResponse {
  cancelled: boolean
}
