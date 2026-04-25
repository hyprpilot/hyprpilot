/**
 * Tiny ANSI subset stripper. Pilot's terminal card honored color
 * escapes (`\x1b[<n>m`) as inert noise and `\x1b[2K` (clear-line)
 * as "drop the current line" — that's the floor. We strip color
 * escapes (no rendering today; revisit when a real terminal
 * primitive lands), then collapse `\x1b[2K` plus the preceding
 * line content so the visible scrollback matches what a user
 * would see in a real terminal.
 *
 * Anything else in the ANSI namespace passes through. We
 * deliberately do NOT pull `ansi_up` or `ansi-to-html` — those
 * weigh ~50 KB and pull a brittle parser tree we don't need.
 */

const COLOR_ESCAPE_RE = /\x1b\[[0-9;]*m/g

/** `\x1b[2K` clears the current line (cursor stays). Drop the current logical line. */
const CLEAR_LINE_RE = /[^\n]*\x1b\[2K/g

export function stripAnsi(input: string): string {
  if (!input) {
    return ''
  }

  return input.replace(CLEAR_LINE_RE, '').replace(COLOR_ESCAPE_RE, '')
}
