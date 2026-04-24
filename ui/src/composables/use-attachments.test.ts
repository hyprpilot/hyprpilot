import { afterEach, describe, expect, it } from 'vitest'

import { useAttachments } from './use-attachments'

const SAMPLE = {
  slug: 'debug-rust',
  path: '/skills/debug-rust.md',
  body: '# debug rust\n\nstep 1',
  title: 'Debug Rust'
}

afterEach(() => {
  useAttachments().clear()
})

describe('useAttachments', () => {
  it('add() appends a new attachment', () => {
    const a = useAttachments()
    a.add(SAMPLE)
    expect(a.pending.value).toHaveLength(1)
    expect(a.pending.value[0]?.slug).toBe('debug-rust')
  })

  it('add() dedupes on slug — second add is a no-op', () => {
    const a = useAttachments()
    a.add(SAMPLE)
    a.add({ ...SAMPLE, body: 'replaced body' })
    expect(a.pending.value).toHaveLength(1)
    expect(a.pending.value[0]?.body).toBe('# debug rust\n\nstep 1')
  })

  it('remove() drops the matching slug', () => {
    const a = useAttachments()
    a.add(SAMPLE)
    a.add({ ...SAMPLE, slug: 'other' })
    a.remove('debug-rust')
    expect(a.pending.value).toHaveLength(1)
    expect(a.pending.value[0]?.slug).toBe('other')
  })

  it('clear() empties the list', () => {
    const a = useAttachments()
    a.add(SAMPLE)
    a.add({ ...SAMPLE, slug: 'other' })
    a.clear()
    expect(a.pending.value).toEqual([])
  })

  it('has() reflects membership by slug', () => {
    const a = useAttachments()
    expect(a.has('debug-rust')).toBe(false)
    a.add(SAMPLE)
    expect(a.has('debug-rust')).toBe(true)
    a.remove('debug-rust')
    expect(a.has('debug-rust')).toBe(false)
  })

  it('module-singleton: separate calls share the same store', () => {
    useAttachments().add(SAMPLE)
    expect(useAttachments().pending.value).toHaveLength(1)
    expect(useAttachments().has('debug-rust')).toBe(true)
  })
})
