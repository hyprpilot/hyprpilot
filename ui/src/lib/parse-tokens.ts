/**
 * UI mirror of the daemon's generic token-hydration parser
 * (`src-tauri/src/adapters/tokens.rs`). Walks every
 * `#{<scheme>://<value>}` token in the source text and emits a
 * `ParsedToken` per match. Used by the dev-preview shim + vitest
 * fixtures to project inline tokens back to their (scheme, value)
 * pairs for display.
 *
 * Production submit goes through the daemon's `session_submit`
 * Tauri command, which runs the authoritative hydration via the
 * registered `TokenHydrator`s. Adding a new scheme is a daemon-side
 * concern; this parser is scheme-agnostic and stays the same.
 */

const TOKEN_PATTERN = /#\{([a-z][a-z0-9_-]*):\/\/([^}]*)\}/g

export interface ParsedToken {
  scheme: string
  value: string
  /** Byte offsets in the source text where the token starts / ends. */
  start: number
  end: number
}

export function parseTokens(text: string): ParsedToken[] {
  const out: ParsedToken[] = []
  let match: RegExpExecArray | null
  while ((match = TOKEN_PATTERN.exec(text)) !== null) {
    out.push({
      scheme: match[1] ?? '',
      value: match[2] ?? '',
      start: match.index,
      end: match.index + match[0].length
    })
  }
  return out
}
