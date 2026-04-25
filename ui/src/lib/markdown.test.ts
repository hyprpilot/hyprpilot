import { describe, expect, it } from 'vitest'

import { escapeHtml, renderMarkdown } from '@lib'

describe('renderMarkdown', () => {
  it('applies target=_blank and rel=noopener noreferrer to auto-linked URLs', async () => {
    const { html } = await renderMarkdown('see https://example.com for details')
    expect(html).toContain('target="_blank"')
    expect(html).toContain('rel="noopener noreferrer"')
  })

  it('applies target=_blank and rel=noopener noreferrer to explicit markdown links', async () => {
    const { html } = await renderMarkdown('[link](https://example.com)')
    expect(html).toContain('target="_blank"')
    expect(html).toContain('rel="noopener noreferrer"')
    expect(html).toContain('href="https://example.com"')
  })

  it('escapes raw <script> tags so they never reach the DOM as a parser-active element', async () => {
    const { html } = await renderMarkdown('<script>alert(1)</script>')
    expect(html).not.toContain('<script>')
    expect(html).not.toContain('</script>')
    // The escaped form (text content) is fine; what matters is no live tag.
    expect(html).toContain('&lt;script&gt;')
  })

  it('renders GFM tables', async () => {
    const src = ['| col a | col b |', '| --- | --- |', '| 1 | 2 |'].join('\n')
    const { html } = await renderMarkdown(src)
    expect(html).toContain('<table>')
    expect(html).toContain('<thead>')
    expect(html).toContain('<th>col a</th>')
    expect(html).toContain('<td>2</td>')
  })

  it('renders task list items as input checkboxes', async () => {
    const src = '- [x] done\n- [ ] todo'
    const { html } = await renderMarkdown(src)
    expect(html).toContain('class="task-list-item-checkbox"')
    expect(html).toContain('checked')
    expect(html).toContain('done')
    expect(html).toContain('todo')
  })

  it('renders strikethrough', async () => {
    const { html } = await renderMarkdown('~~gone~~')
    expect(html).toContain('<s>')
    expect(html).toContain('gone')
  })

  it('inline code renders as <code>', async () => {
    const { html } = await renderMarkdown('see `foo()` for details')
    expect(html).toContain('<code>foo()</code>')
  })

  it('fenced code with a known language flows through Shiki (token spans appear)', async () => {
    const src = '```ts\nconst x: number = 1\n```'
    const { html } = await renderMarkdown(src)
    // Shiki emits a `<pre class="shiki ...">` wrapper plus inline-styled
    // `<span>`s per token.
    expect(html).toContain('<pre')
    expect(html).toContain('class="shiki')
    expect(html).toMatch(/<span style="[^"]*--shiki-/)
  })

  it('fenced code with an unknown language falls through to <pre><code>', async () => {
    const src = '```definitely-not-a-real-language\nplain code\n```'
    const { html } = await renderMarkdown(src)
    expect(html).toContain('<pre>')
    expect(html).toContain('<code>')
    expect(html).toContain('plain code')
    // No Shiki tokens for an unknown lang.
    expect(html).not.toContain('--shiki-')
  })

  it('strips javascript: hrefs from explicit markdown links', async () => {
    // markdown-it requires an http-ish-looking URL for autolink, so use
    // explicit `[label](href)` form — that path keeps the literal href
    // until the sanitiser drops it.
    const { html } = await renderMarkdown('[click](javascript://alert%281%29)')
    expect(html).toContain('click')
    expect(html.toLowerCase()).not.toMatch(/href="javascript:/i)
  })

  it('escapes inline event-handler attributes inside raw HTML so they never become live attrs', async () => {
    const { html } = await renderMarkdown('<a href="#" onclick="bad()">x</a>')
    // `html: false` escapes the whole thing to text. What matters is
    // there's no live attribute named onclick on a real element.
    expect(html).not.toMatch(/<a[^>]*\sonclick=/i)
  })

  it('injects a copy button per fenced code block', async () => {
    const src = '```ts\nconst x = 1\n```\n\n```sh\necho hi\n```'
    const { html } = await renderMarkdown(src)
    const copyMatches = html.match(/data-md-copy/g) ?? []
    expect(copyMatches).toHaveLength(2)
    expect(html).toContain('class="md-codeblock"')
  })
})

describe('escapeHtml', () => {
  it('escapes the four dangerous characters', () => {
    expect(escapeHtml('<img src=x onerror=bad>')).toBe('&lt;img src=x onerror=bad&gt;')
    expect(escapeHtml('"quoted"')).toBe('&quot;quoted&quot;')
    expect(escapeHtml('a & b')).toBe('a &amp; b')
  })
})
