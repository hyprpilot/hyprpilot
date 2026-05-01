import { beforeEach, describe, expect, it } from 'vitest'

import { ComposerPillKind } from '@components'

import { __resetComposerForTests, useComposer } from './use-composer'

describe('useComposer', () => {
  beforeEach(() => {
    __resetComposerForTests()
  })

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

  it('useComposer() returns the same module-scope state across calls', () => {
    const a = useComposer()
    const b = useComposer()
    a.text.value = 'shared'
    expect(b.text.value).toBe('shared')
  })

  it('insertAtCaret with no registered textarea appends to the buffer', () => {
    const c = useComposer()
    c.text.value = 'hello'
    c.insertAtCaret(' world')
    expect(c.text.value).toBe('hello world')
  })

  it('insertAtCaret with a registered textarea splices at the selection', () => {
    const c = useComposer()
    c.text.value = 'hello world'
    const ta = document.createElement('textarea')
    document.body.appendChild(ta)
    try {
      ta.value = 'hello world'
      ta.setSelectionRange(5, 5)
      c.registerTextarea(ta)

      c.insertAtCaret(' beautiful')
      expect(c.text.value).toBe('hello beautiful world')
    } finally {
      c.registerTextarea(undefined)
      ta.remove()
    }
  })

  it('insertAtCaret with a selected range replaces the selection', () => {
    const c = useComposer()
    c.text.value = 'hello world'
    const ta = document.createElement('textarea')
    document.body.appendChild(ta)
    try {
      ta.value = 'hello world'
      ta.setSelectionRange(0, 5)
      c.registerTextarea(ta)

      c.insertAtCaret('greetings')
      expect(c.text.value).toBe('greetings world')
    } finally {
      c.registerTextarea(undefined)
      ta.remove()
    }
  })
})
