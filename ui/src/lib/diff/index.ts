/**
 * Diff helpers — produce unified-diff text from `(oldText, newText)`
 * pairs and emit two render shapes:
 *
 * - **Cheap** — fenced ` ```diff ` block. Shiki's built-in `diff`
 *   grammar tokenises `+` / `-` / `@@` markers; +/- only, no
 *   per-language syntax. Used in the spec-sheet's description body
 *   on post-completion edit pills (review path, high volume).
 * - **Rich** — per-language source with `// [!code ++]` /
 *   `// [!code --]` comment annotations injected at the changed
 *   lines. Renders via Shiki + `transformerNotationDiff` from
 *   `@shikijs/transformers`; keeps full language syntax highlighting
 *   AND adds gutter `+` / `-` styling. Used inside the permission
 *   modal (decision path, captain wants high-fidelity diff).
 *
 * Wiring: the modal body consumes `richDiffMarkdown` via
 * `<MarkdownBody>` (Shiki transformers run inside the markdown
 * pipeline); the spec sheet consumes `cheapDiffMarkdown` the same
 * way.
 */

import { createPatch, structuredPatch } from 'diff'

import { resolveShikiLang } from '@lib/markdown/mime'

/**
 * Cheap rendering — produce a fenced ` ```diff ` markdown block from
 * a unified diff. Shiki's built-in `diff` grammar tokenises the
 * resulting fence; no transformer dependency.
 *
 * `path` is shown on the `--- ` / `+++ ` headers so the diff reads
 * as a real patch the captain could pipe through `patch -p1`.
 */
export function cheapDiffMarkdown(path: string, oldText: string, newText: string): string {
  const patch = createPatch(path, oldText, newText, '', '', { context: 3 })

  return '```diff\n' + patch + '\n```'
}

/**
 * Rich rendering — produce a per-language source with diff markers
 * injected as `// [!code ++]` / `// [!code --]` annotations at the
 * changed lines. The marker comment style varies by language: most
 * code uses `//`, but `python` / `bash` / `yaml` / `toml` use `#`,
 * `html` / `xml` use `<!-- … -->`, etc. Falls back to a plain
 * unified diff when the language isn't recognised — Shiki still
 * highlights, just without the diff transformer.
 */
export function richDiffMarkdown(path: string, mime: string | undefined, oldText: string, newText: string): { source: string; lang: string } {
  const lang = resolveShikiLang(mime, path) ?? 'plaintext'
  const comment = commentStyleForLang(lang)

  // No comment style for the language → fall back to cheap unified
  // diff. transformerNotationDiff would no-op anyway without
  // language-comment annotations.
  if (!comment) {
    return { source: cheapDiffMarkdown(path, oldText, newText), lang: 'diff' }
  }

  const hunks = structuredPatch(path, path, oldText, newText, '', '', { context: 3 })
  const out: string[] = []

  for (const hunk of hunks.hunks) {
    for (const line of hunk.lines) {
      // Each hunk line starts with ' ' (context), '+' (add), or '-' (remove).
      const tag = line.charAt(0)
      const body = line.slice(1)

      if (tag === '+') {
        out.push(`${body} ${comment} [!code ++]`)
      } else if (tag === '-') {
        out.push(`${body} ${comment} [!code --]`)
      } else {
        // Context line — render as-is, no marker.
        out.push(body)
      }
    }
  }
  const source = '```' + lang + '\n' + out.join('\n') + '\n```'

  return { source, lang }
}

/**
 * Comment-style table for `transformerNotationDiff` markers. Languages
 * where line-comment markers can't sit at end-of-line without
 * breaking parsing (HTML / XML / Markdown / plaintext) are absent —
 * the caller then falls back to the cheap unified-diff path.
 */
const commentStyles = new Map<string, string>([
  ['typescript', '//'],
  ['javascript', '//'],
  ['rust', '//'],
  ['go', '//'],
  ['java', '//'],
  ['kotlin', '//'],
  ['swift', '//'],
  ['csharp', '//'],
  ['cpp', '//'],
  ['c', '//'],
  ['css', '//'],
  ['scss', '//'],
  ['sass', '//'],
  ['json', '//'],
  ['jsonc', '//'],
  ['vue', '//'],
  ['python', '#'],
  ['bash', '#'],
  ['shell', '#'],
  ['sh', '#'],
  ['zsh', '#'],
  ['fish', '#'],
  ['yaml', '#'],
  ['toml', '#'],
  ['r', '#'],
  ['ruby', '#'],
  ['perl', '#'],
  ['makefile', '#'],
  ['dockerfile', '#'],
  ['sql', '--'],
  ['lua', '--'],
  ['haskell', '--']
])

function commentStyleForLang(lang: string): string | undefined {
  return commentStyles.get(lang)
}
