import MarkdownIt from 'markdown-it'
import { codeToHtml, type BundledLanguage, type BundledTheme } from 'shiki'

const SHIKI_THEME: BundledTheme = 'github-dark-default'

const shikiCache = new Map<string, string>()

/**
 * Shiki is async; `markdown-it`'s `highlight` hook is sync. We warm
 * the cache out-of-band and return the raw escaped block until the
 * first render; the second render after the cache fills shows syntax
 * highlighting. Good enough for a streaming overlay — re-rendering
 * on every chunk already happens.
 */
function syncHighlight(md: MarkdownIt, code: string, lang: string): string {
  const key = `${lang}\u0000${code}`
  const cached = shikiCache.get(key)
  if (cached) {
    return cached
  }

  void codeToHtml(code, { lang: lang as BundledLanguage, theme: SHIKI_THEME })
    .then((html) => {
      shikiCache.set(key, html)
    })
    .catch(() => {
      // unknown lang: stash the escaped version so we stop retrying
      shikiCache.set(key, `<pre><code>${md.utils.escapeHtml(code)}</code></pre>`)
    })

  return `<pre><code>${md.utils.escapeHtml(code)}</code></pre>`
}

const md = new MarkdownIt({
  html: false,
  linkify: true,
  breaks: true,
  highlight: (code: string, lang: string): string => {
    if (!lang) {
      return `<pre><code>${md.utils.escapeHtml(code)}</code></pre>`
    }

    return syncHighlight(md, code, lang)
  }
})

// linkify-emitted <a> tags default to same-tab navigation; pin every link
// to a new tab with `noopener noreferrer` so markdown URLs cannot steal the
// overlay window or leak Referer headers.
const defaultLinkOpen = md.renderer.rules.link_open ?? ((tokens, idx, opts, _env, self) => self.renderToken(tokens, idx, opts))
md.renderer.rules.link_open = (tokens, idx, opts, env, self) => {
  const token = tokens[idx]
  token.attrSet('target', '_blank')
  token.attrSet('rel', 'noopener noreferrer')

  return defaultLinkOpen(tokens, idx, opts, env, self)
}

/** Renders markdown to trusted HTML. `html: false` in markdown-it keeps raw `<script>` out. */
export function renderMarkdown(src: string): string {
  return md.render(src)
}

/** HTML-escapes a raw string (delegates to markdown-it's util). Safe to splice into `v-html`. */
export const escapeHtml: (s: string) => string = md.utils.escapeHtml
