import { describe, expect, it } from 'vitest'

import { escapeHtml, renderMarkdown } from '@lib'

describe('renderMarkdown', () => {
  it('applies target=_blank and rel=noopener noreferrer to auto-linked URLs', () => {
    const html = renderMarkdown('see https://example.com for details')
    expect(html).toContain('target="_blank"')
    expect(html).toContain('rel="noopener noreferrer"')
  })

  it('applies target=_blank and rel=noopener noreferrer to explicit markdown links', () => {
    const html = renderMarkdown('[link](https://example.com)')
    expect(html).toContain('target="_blank"')
    expect(html).toContain('rel="noopener noreferrer"')
    expect(html).toContain('href="https://example.com"')
  })

  it('still strips raw HTML blocks (html: false)', () => {
    const html = renderMarkdown('<script>alert(1)</script>')
    expect(html).not.toContain('<script>')
    expect(html).toContain('&lt;script&gt;')
  })
})

describe('escapeHtml', () => {
  it('escapes the four dangerous characters', () => {
    expect(escapeHtml('<img src=x onerror=bad>')).toBe('&lt;img src=x onerror=bad&gt;')
    expect(escapeHtml('"quoted"')).toBe('&quot;quoted&quot;')
    expect(escapeHtml('a & b')).toBe('a &amp; b')
  })
})
