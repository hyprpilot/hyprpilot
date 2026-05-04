/**
 * MIME → Shiki language id lookup. Closed set covering the dozen
 * common code MIMEs the daemon emits via `mime_guess` / HTTP
 * `Content-Type`. Unknown / unmapped MIMEs return `undefined`; the
 * caller falls back to a plain `<pre>` (or `lang: 'plaintext'`).
 *
 * Kept tiny + boring — Shiki's full bundled-language list is huge;
 * loading every grammar at boot is wasteful for the captain's
 * common case (TS / JSON / Markdown / YAML / Rust / shell). New
 * extensions land here as captains hit them.
 */

// `Map` instead of an object literal to dodge the camelCase
// property-name lint rule — MIME types carry slashes that can't be
// expressed as identifiers.
const mimeToLang = new Map<string, string>([
  ['application/json', 'json'],
  ['application/x-json', 'json'],
  ['application/javascript', 'javascript'],
  ['application/x-javascript', 'javascript'],
  ['text/javascript', 'javascript'],
  ['application/typescript', 'typescript'],
  ['application/x-typescript', 'typescript'],
  ['text/typescript', 'typescript'],
  ['application/xml', 'xml'],
  ['text/xml', 'xml'],
  ['application/x-yaml', 'yaml'],
  ['application/yaml', 'yaml'],
  ['text/yaml', 'yaml'],
  ['text/x-yaml', 'yaml'],
  ['application/toml', 'toml'],
  ['text/x-toml', 'toml'],
  ['text/markdown', 'markdown'],
  ['text/x-markdown', 'markdown'],
  ['text/html', 'html'],
  ['text/css', 'css'],
  ['text/x-rust', 'rust'],
  ['application/x-rust', 'rust'],
  ['text/x-go', 'go'],
  ['text/x-python', 'python'],
  ['application/x-python', 'python'],
  ['text/x-shellscript', 'bash'],
  ['application/x-sh', 'bash'],
  ['application/x-shellscript', 'bash'],
  ['text/x-sql', 'sql'],
  ['application/sql', 'sql'],
  ['text/x-vue', 'vue']
])

const extToLang = new Map<string, string>([
  ['ts', 'typescript'],
  ['tsx', 'typescript'],
  ['cts', 'typescript'],
  ['mts', 'typescript'],
  ['js', 'javascript'],
  ['jsx', 'javascript'],
  ['cjs', 'javascript'],
  ['mjs', 'javascript'],
  ['json', 'json'],
  ['jsonc', 'json'],
  ['md', 'markdown'],
  ['markdown', 'markdown'],
  ['yaml', 'yaml'],
  ['yml', 'yaml'],
  ['toml', 'toml'],
  ['rs', 'rust'],
  ['go', 'go'],
  ['py', 'python'],
  ['sh', 'bash'],
  ['bash', 'bash'],
  ['zsh', 'bash'],
  ['sql', 'sql'],
  ['vue', 'vue'],
  ['html', 'html'],
  ['htm', 'html'],
  ['css', 'css'],
  ['xml', 'xml']
])

export function mimeToShikiLang(mime: string | undefined): string | undefined {
  if (!mime) {
    return undefined
  }
  // Strip charset / boundary parameters (`text/plain; charset=utf-8`).
  const base = mime.split(';')[0]?.trim().toLowerCase()

  if (!base || base === 'text/plain') {
    return undefined
  }

  return mimeToLang.get(base)
}

/**
 * Last-resort language inference from a path's extension. Backend
 * `mime_guess` should populate `mime` for most files; this falls
 * through for unusual extensions or path-only paths.
 */
export function pathToShikiLang(path: string | undefined): string | undefined {
  if (!path) {
    return undefined
  }
  const seg = path.split('/').pop() ?? ''
  const dot = seg.lastIndexOf('.')

  if (dot < 0 || dot === seg.length - 1) {
    return undefined
  }
  const ext = seg.slice(dot + 1).toLowerCase()

  return extToLang.get(ext)
}

/**
 * Combined MIME-or-path resolution. MIME wins when it maps; path
 * extension is the fallback. `undefined` means "render as a plain
 * `<pre>` without language tagging".
 */
export function resolveShikiLang(mime: string | undefined, path: string | undefined): string | undefined {
  return mimeToShikiLang(mime) ?? pathToShikiLang(path)
}
