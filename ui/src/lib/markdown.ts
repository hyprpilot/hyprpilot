import DOMPurify from 'dompurify'
import MarkdownIt from 'markdown-it'
import taskLists from 'markdown-it-task-lists'
import { createHighlighter, type BundledLanguage, type BundledTheme, type Highlighter } from 'shiki'

import { log } from './log'
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
      const hl = await getHighlighter()

      await hl.loadLanguage(lang as BundledLanguage)
      loadedLangs.add(lang)

      return true
    } catch(err) {
      if (!warnedLangs.has(lang)) {
        warnedLangs.add(lang)
        log.warn('shiki: unknown language; falling back to plain code block', { lang, err: String(err) })
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

async function highlightFence(code: string, lang: string): Promise<string> {
  if (!lang) {
    return fallbackCode(code)
  }
  const ok = await ensureLanguage(lang)

  if (!ok) {
    return fallbackCode(code)
  }

  try {
    const hl = await getHighlighter()

    return hl.codeToHtml(code, { lang: lang as BundledLanguage, theme: resolvedTheme })
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

export const escapeHtml: (s: string) => string = md.utils.escapeHtml
