import DOMPurify from 'dompurify'
import MarkdownIt from 'markdown-it'
import taskLists from 'markdown-it-task-lists'
import { createHighlighter, type BundledLanguage, type BundledTheme, type Highlighter, type ShikiTransformer } from 'shiki'

import { log } from '../log'
import { invoke, TauriCommand } from '@ipc'

export interface RenderedMarkdown {
  html: string
}

const DEFAULT_THEME: BundledTheme = 'one-dark-pro'

let highlighterPromise: Promise<Highlighter> | undefined
let resolvedTheme: BundledTheme = DEFAULT_THEME
const loadedLangs = new Set<string>()
const langLoading = new Map<string, Promise<boolean>>()
const warnedLangs = new Set<string>()

/**
 * Resolve the Shiki theme name from the daemon's `[ui.theme] shiki`
 * config; soft-fail to `one-dark-pro` when the IPC isn't bound (vitest
 * jsdom). Cached across calls so the highlighter only initialises with
 * one theme. Override at runtime by invoking `setShikiTheme`.
 */
async function resolveShikiTheme(): Promise<BundledTheme> {
  try {
    const theme = await invoke(TauriCommand.GetTheme)
    const name = theme.shiki

    if (typeof name === 'string' && name.length > 0) {
      return name as BundledTheme
    }
  } catch {
    // No Tauri host (vitest) — fall through to the default.
  }

  return DEFAULT_THEME
}

function getHighlighter(): Promise<Highlighter> {
  if (!highlighterPromise) {
    highlighterPromise = (async() => {
      resolvedTheme = await resolveShikiTheme()

      return createHighlighter({
        themes: [resolvedTheme],
        langs: []
      })
    })()
  }

  return highlighterPromise
}

/// Pre-warm Shiki at module load. The highlighter init does
/// dynamic ESM imports for the WASM oniguruma engine; in the Tauri
/// WebKitGTK runtime those imports occasionally take seconds to
/// resolve and the FIRST `renderMarkdown` call waits on them. By
/// firing init at import time the highlighter is usually ready by
/// the time any UI code renders markdown — so the first permission
/// modal doesn't get stuck on the plain-pass output.
///
/// Errors swallowed: if init fails, subsequent `getHighlighter()`
/// calls hit the same rejected promise and `ensureLanguage` returns
/// false, falling through to the plain code path. No `unhandled
/// rejection` either way.
void getHighlighter().catch(() => undefined)

/// Hard timeout the highlighter init at 3s. Tauri WebKitGTK has
/// shipped builds where the WASM oniguruma engine's module import
/// promise never resolves OR rejects (silent stall). Without a
/// timeout, every `renderMarkdown` call awaits the dead promise
/// forever — MarkdownBody's plain-pass paint sticks because
/// `html.value = out.html` never runs.
const HIGHLIGHTER_TIMEOUT_MS = 3_000

function getHighlighterWithTimeout(): Promise<Highlighter> {
  return Promise.race([
    getHighlighter(),
    new Promise<Highlighter>((_, reject) => {
      setTimeout(() => reject(new Error(`shiki: highlighter init exceeded ${HIGHLIGHTER_TIMEOUT_MS}ms`)), HIGHLIGHTER_TIMEOUT_MS)
    })
  ])
}

async function ensureLanguage(lang: string): Promise<boolean> {
  if (!lang) {
    return false
  }

  if (loadedLangs.has(lang)) {
    return true
  }
  const pending = langLoading.get(lang)

  if (pending) {
    return pending
  }

  const task = (async(): Promise<boolean> => {
    try {
      const hl = await getHighlighterWithTimeout()

      await Promise.race([
        hl.loadLanguage(lang as BundledLanguage),
        new Promise<void>((_, reject) => {
          setTimeout(() => reject(new Error(`shiki: loadLanguage('${lang}') exceeded ${HIGHLIGHTER_TIMEOUT_MS}ms`)), HIGHLIGHTER_TIMEOUT_MS)
        })
      ])
      loadedLangs.add(lang)

      return true
    } catch(err) {
      if (!warnedLangs.has(lang)) {
        warnedLangs.add(lang)
        log.warn('shiki: language load failed; falling back to plain code block', { lang, err: String(err) })
      }

      return false
    } finally {
      langLoading.delete(lang)
    }
  })()

  langLoading.set(lang, task)

  return task
}

function fallbackCode(code: string): string {
  return `<pre><code>${md.utils.escapeHtml(code)}</code></pre>`
}

/// Match `// [!code ++/--]` (and `#`, `--`, `;` comment-style
/// variants) at line end — the shape `format_diff_hunk` emits.
const DIFF_MARKER_RE = /\s*(?:\/\/|#|--|;)\s*\[!code (\+\+|--)\]\s*$/

/// Custom Shiki transformer that adds `.diff.add` / `.diff.remove`
/// classes to lines carrying our markers and strips the markers
/// from the rendered text. Replaces `@shikijs/transformers`'s
/// `transformerNotationDiff` because that one walks per-token AST
/// nodes inside each line and depends on Shiki's grammar tokenising
/// the marker comment as the line's last element — fragile across
/// runtimes (vitest jsdom passes; Tauri's WebKitGTK2 webview
/// silently misses some token shapes, leaving every fence
/// unhighlighted with markers visible). Walking the line's plain-text
/// concatenation is grammar-independent and reproducible.
const diffMarkerTransformer = (): ShikiTransformer => {
  let active = false
  let warned = false

  return {
    name: 'hyprpilot:diff-marker',
    line(node) {
      try {
        let text = ''
        const collect = (n: { children?: unknown[]; type?: string; value?: string }): void => {
          if (n.type === 'text' && typeof n.value === 'string') {
            text += n.value
          }

          if (Array.isArray(n.children)) {
            for (const child of n.children) {
              collect(child as { children?: unknown[]; type?: string; value?: string })
            }
          }
        }

        collect(node as unknown as { children?: unknown[]; type?: string; value?: string })

        const m = DIFF_MARKER_RE.exec(text)

        if (!m) {
          return
        }
        active = true
        const cls = m[1] === '++' ? 'diff add' : 'diff remove'

        if (node.properties === undefined) {
          node.properties = {}
        }
        const props = node.properties as Record<string, unknown>
        const existing = typeof props.class === 'string' ? props.class : ''

        props.class = existing ? `${existing} ${cls}` : cls

        // Strip the marker substring from whichever text node carries it.
        const strip = (n: { children?: unknown[]; type?: string; value?: string }): boolean => {
          if (n.type === 'text' && typeof n.value === 'string' && DIFF_MARKER_RE.test(n.value)) {
            n.value = n.value.replace(DIFF_MARKER_RE, '')

            return true
          }

          if (Array.isArray(n.children)) {
            for (const child of n.children) {
              if (strip(child as { children?: unknown[]; type?: string; value?: string })) {
                return true
              }
            }
          }

          return false
        }

        strip(node as unknown as { children?: unknown[]; type?: string; value?: string })
      } catch(err) {
        if (!warned) {
          warned = true
          log.warn('shiki diff transformer: line hook failed; skipping', { err: String(err) })
        }
      }
    },
    pre(node) {
      try {
        if (!active) {
          return
        }

        if (node.properties === undefined) {
          node.properties = {}
        }
        const props = node.properties as Record<string, unknown>
        const existing = typeof props.class === 'string' ? props.class : ''

        props.class = existing ? `${existing} has-diff` : 'has-diff'
      } catch(err) {
        if (!warned) {
          warned = true
          log.warn('shiki diff transformer: pre hook failed; skipping', { err: String(err) })
        }
      }
    }
  }
}

async function highlightFence(code: string, lang: string): Promise<string> {
  if (!lang) {
    return fallbackCode(code)
  }
  const ok = await ensureLanguage(lang)

  if (!ok) {
    return fallbackCode(code)
  }

  try {
    const hl = await getHighlighterWithTimeout()

    return hl.codeToHtml(code, {
      lang: lang as BundledLanguage,
      theme: resolvedTheme,
      transformers: [diffMarkerTransformer()]
    })
  } catch(err) {
    log.warn('shiki: codeToHtml failed; falling back', { lang, err: String(err) })

    return fallbackCode(code)
  }
}

const FENCE_PLACEHOLDER_RE = /<pre data-hp-fence-idx="(\d+)"><\/pre>/g

interface PendingFence {
  code: string
  lang: string
}

const md = new MarkdownIt({
  html: false,
  linkify: true,
  breaks: true
})

md.use(taskLists, { enabled: false, label: false })

const defaultLinkOpen = md.renderer.rules.link_open ?? ((tokens, idx, opts, _env, self) => self.renderToken(tokens, idx, opts))

md.renderer.rules.link_open = (tokens, idx, opts, env, self) => {
  const token = tokens[idx]

  token.attrSet('target', '_blank')
  token.attrSet('rel', 'noopener noreferrer')

  return defaultLinkOpen(tokens, idx, opts, env, self)
}

/**
 * Pre-render pass: replace each fence with a unique placeholder so
 * markdown-it's `highlight` hook (sync) can stash the (code, lang)
 * for an async highlight pass; the placeholder swaps for highlighted
 * HTML after the sanitiser runs.
 */
function withFencePlaceholders(): { fences: PendingFence[]; restore: () => void } {
  const fences: PendingFence[] = []
  const prevHighlight = md.options.highlight

  md.set({
    highlight: (code: string, lang: string): string => {
      const idx = fences.length

      fences.push({ code, lang: lang.trim() })

      // markdown-it's default fence renderer takes a `<pre`-prefixed
      // return verbatim — the whole tag is the post-render fence slot,
      // and we substitute it with the highlighted output below.
      return `<pre data-hp-fence-idx="${idx}"></pre>`
    }
  })

  return {
    fences,
    restore: () => {
      md.set({ highlight: prevHighlight })
    }
  }
}

DOMPurify.addHook('uponSanitizeAttribute', (_node, data) => {
  if (data.attrName === 'href' || data.attrName === 'src') {
    if (/^\s*javascript:/i.test(data.attrValue)) {
      data.keepAttr = false
    }
  }
})

function sanitize(html: string): string {
  // USE_PROFILES.html seeds a sane HTML allowlist; ADD_ATTR layers in the
  // bits the html profile excludes (target/rel for outbound link
  // policy, our own data-* hooks). target/rel must also be marked
  // URI-safe — DOMPurify otherwise validates their value as a URI and
  // strips them when the value (`_blank`, `noopener noreferrer`)
  // doesn't match ALLOWED_URI_REGEXP.
  return DOMPurify.sanitize(html, {
    USE_PROFILES: { html: true },
    ADD_ATTR: ['target', 'rel', 'data-lang', 'data-md-copy'],
    ADD_URI_SAFE_ATTR: ['target', 'rel'],
    ALLOWED_URI_REGEXP: /^(?:https?|mailto|tel|ftp|file|#):/i,
    FORBID_ATTR: ['onerror', 'onclick', 'onload', 'onmouseover', 'onfocus', 'onblur', 'onsubmit'],
    FORBID_TAGS: ['script', 'iframe', 'object', 'embed', 'form', 'meta', 'link', 'base']
  }) as string
}

interface FenceRenderInput {
  html: string
  lang: string
}

function injectCopyButton({ html, lang }: FenceRenderInput): string {
  const trimmed = html.trim()
  const hasPre = trimmed.startsWith('<pre')
  const inner = hasPre ? trimmed : `<pre><code>${md.utils.escapeHtml(trimmed)}</code></pre>`
  const langAttr = lang ? ` data-lang="${md.utils.escapeHtml(lang)}"` : ''
  const langLabel = lang ? `<span class="md-codeblock-lang">${md.utils.escapeHtml(lang)}</span>` : ''

  // Code-block chrome: caret + lang label + copy button. Caret
  // glyphs are unicode triangles (`▾` / `▸`) — no inlined SVG paths
  // because the markdown pipeline emits HTML, not Vue templates, and
  // mounting `<FaIcon>` post-`v-html` is more friction than a 1-char
  // unicode glyph buys. Body.vue's scoped CSS picks the right caret
  // via `[data-collapsed]`. Copy button stays a plain text label —
  // the operator vibe (mono font, lowercase) reads better than a
  // glyph in this context.
  return (
    `<div class="md-codeblock" data-collapsed="false"${langAttr}>` +
    '<header class="md-codeblock-header" data-md-toggle role="button" tabindex="0" aria-label="toggle code block">' +
    '<span class="md-codeblock-caret" data-md-caret-down>▾</span>' +
    '<span class="md-codeblock-caret" data-md-caret-right>▸</span>' +
    langLabel +
    '<span class="md-codeblock-spacer"></span>' +
    '<button type="button" class="md-copy" data-md-copy aria-label="copy code" title="copy">copy</button>' +
    '</header>' +
    `<div class="md-codeblock-body">${inner}</div>` +
    '</div>'
  )
}

/**
 * Renders markdown to sanitised HTML. The result is safe to drop into
 * `v-html`. Fenced code blocks pass through Shiki under the bundled
 * theme configured in Rust (`[ui.theme] shiki`); unknown languages
 * fall back to `<pre><code>` with the raw text.
 */
export async function renderMarkdown(src: string): Promise<RenderedMarkdown> {
  const { fences, restore } = withFencePlaceholders()
  let raw: string

  try {
    raw = md.render(src)
  } finally {
    restore()
  }

  const highlighted = await Promise.all(fences.map((f) => highlightFence(f.code, f.lang)))
  const wrapped = highlighted.map((html, i) => injectCopyButton({ html, lang: fences[i]?.lang ?? '' }))

  const stitched = raw.replace(FENCE_PLACEHOLDER_RE, (_match, idxStr: string) => {
    const idx = Number.parseInt(idxStr, 10)

    return wrapped[idx] ?? fallbackCode(fences[idx]?.code ?? '')
  })

  return { html: sanitize(stitched) }
}

/**
 * Synchronous markdown render WITHOUT Shiki. Code fences land as
 * `<pre><code>` (HTML-escaped) wrapped in the `injectCopyButton`
 * chrome. Diff markers (`// [!code ++/--]` shape from rich diff
 * hunks) are rewritten to `+ ` / `- ` line prefixes so the diff
 * direction stays readable. Used as MarkdownBody's safety-net when
 * Shiki throws or stalls — captains never see the raw triple-
 * backtick markdown source as a `<pre>` text dump.
 */
export function renderMarkdownPlain(src: string): string {
  const { fences, restore } = withFencePlaceholders()
  let raw: string

  try {
    raw = md.render(src)
  } finally {
    restore()
  }

  const stitched = raw.replace(FENCE_PLACEHOLDER_RE, (_match, idxStr: string) => {
    const idx = Number.parseInt(idxStr, 10)
    const fence = fences[idx]

    if (!fence) {
      return ''
    }
    const rewritten = rewriteDiffMarkersToPrefix(fence.code)

    return injectCopyButton({ html: fallbackCode(rewritten), lang: fence.lang })
  })

  return sanitize(stitched)
}

/// Rewrite `// [!code ++/--]` markers to literal `+ ` / `- ` line
/// prefixes for the no-Shiki fallback path. Drops the marker text;
/// preserves the line content.
function rewriteDiffMarkersToPrefix(code: string): string {
  return code
    .split('\n')
    .map((line) => {
      const m = DIFF_MARKER_RE.exec(line)

      if (!m) {
        return line
      }
      const head = line.slice(0, m.index)
      const prefix = m[1] === '++' ? '+ ' : '- '

      return `${prefix}${head}`
    })
    .join('\n')
}

export const escapeHtml: (s: string) => string = md.utils.escapeHtml
