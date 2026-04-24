import { describe, expect, it } from 'vitest'

import { ComposerPillKind } from '@components/types'

import { useComposer } from './use-composer'

describe('useComposer', () => {
  it('addPill appends; duplicate ids are ignored', () => {
    const c = useComposer()
    c.addPill({ kind: ComposerPillKind.Attachment, id: 'a', label: 'image/png · 1KB', data: 'AA==', mimeType: 'image/png' })
    c.addPill({ kind: ComposerPillKind.Attachment, id: 'a', label: 'image/png · 1KB', data: 'AA==', mimeType: 'image/png' })
    expect(c.pills.value).toHaveLength(1)

    c.addPill({ kind: ComposerPillKind.Attachment, id: 'b', label: 'image/png', data: 'BB==', mimeType: 'image/png' })
    expect(c.pills.value).toHaveLength(2)
  })

  it('removePill drops the matching entry', () => {
    const c = useComposer()
    c.addPill({ kind: ComposerPillKind.Attachment, id: 'a', label: 'image/png', data: 'AA==', mimeType: 'image/png' })
    c.addPill({ kind: ComposerPillKind.Attachment, id: 'b', label: 'image/png', data: 'BB==', mimeType: 'image/png' })

    c.removePill('a')
    expect(c.pills.value.map((p) => p.id)).toEqual(['b'])
  })

  it('clear empties text + pills', () => {
    const c = useComposer()
    c.text.value = 'hello'
    c.addPill({ kind: ComposerPillKind.Attachment, id: 'a', label: 'image/png', data: 'AA==', mimeType: 'image/png' })

    c.clear()
    expect(c.text.value).toBe('')
    expect(c.pills.value).toHaveLength(0)
  })

  it('resolvedSubmit returns trimmed text + image-attachment pills', () => {
    const c = useComposer()
    c.text.value = '  hello world  '
    c.addPill({
      kind: ComposerPillKind.Attachment,
      id: 'img-1',
      label: 'image/png · 1.2KB',
      data: 'AAAA',
      mimeType: 'image/png'
    })

    const { text, attachments } = c.resolvedSubmit()
    expect(text).toBe('hello world')
    expect(attachments).toHaveLength(1)
    expect(attachments[0]?.mimeType).toBe('image/png')
  })

  it('resolvedSubmit no longer expands #{…} tokens (deleted in K-268 pivot)', () => {
    const c = useComposer()
    c.text.value = 'please #{skill/debug} this'

    const { text } = c.resolvedSubmit()
    expect(text).toBe('please #{skill/debug} this')
  })
})
