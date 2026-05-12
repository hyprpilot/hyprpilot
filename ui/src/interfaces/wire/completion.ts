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

/**
 * Closed set of daemon-registered completion sources. Mirrors the
 * Rust `CompletionSource::id()` values in `src-tauri/src/completion/`.
 * Modelled as an enum (not a union literal) so palette modes filter
 * via `[CompletionSourceId.Path]` instead of stringy literals.
 */
export enum CompletionSourceId {
  Skills = 'skills',
  Path = 'path',
  Ripgrep = 'ripgrep',
  Commands = 'commands'
}

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
  /**
   * Whitelist of source ids (`'path'` / `'skills'` / `'commands'` /
   * `'ripgrep'`) the daemon walks during detect. When omitted, every
   * source is eligible. The cwd palette passes `['path']` so its
   * query never gets claimed by skills / commands / ripgrep even
   * when the typed text happens to look like a slash command or
   * hash sigil.
   */
  sources?: CompletionSourceId[]
}

/**
 * One candidate row passed to `completion/rank`. The daemon ranks
 * `label` against the typed query; on commit, the picked row's
 * `id` lands in `CompletionItem.replacement.text` so the caller
 * can route to its own commit handler without re-deriving identity
 * from the rendered label.
 */
export interface CandidateItem {
  id: string
  label: string
  description?: string
}

export interface CompletionQueryResponse {
  requestId: string
  /**
   * `undefined` when no source claimed the cursor position (the
   * daemon returns `sourceId: null` on the wire — `?` is the
   * standard hyprpilot mapping for Rust-side `Option<T>` serialising
   * to `null`).
   */
  sourceId?: CompletionSourceId
  /** `undefined` mirrors a daemon-side `null` — same rationale as `sourceId`. */
  replacementRange?: ReplacementRange
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
