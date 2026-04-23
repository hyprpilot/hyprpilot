import { beforeEach, describe, expect, it } from 'vitest'

import { useActiveInstance } from '@composables/useActiveInstance'

// Module-scoped ref: reset to undefined between tests via the
// exposed set()/setIfUnset() semantics. `setIfUnset('')` is not a
// valid reset because the store stores any truthy string — we
// instead reach through set(undefined as unknown as string) in
// a pre-flight for each test.
beforeEach(() => {
  const { id } = useActiveInstance()
  id.value = undefined
})

describe('useActiveInstance', () => {
  it('first setIfUnset populates the active id', () => {
    const { id, setIfUnset } = useActiveInstance()
    expect(id.value).toBeUndefined()
    setIfUnset('inst-a')
    expect(id.value).toBe('inst-a')
  })

  it('second setIfUnset does not overwrite an existing id', () => {
    const { id, setIfUnset } = useActiveInstance()
    setIfUnset('inst-a')
    setIfUnset('inst-b')
    expect(id.value).toBe('inst-a')
  })

  it('set always overwrites even when the id is already populated', () => {
    const { id, set, setIfUnset } = useActiveInstance()
    setIfUnset('inst-a')
    set('inst-b')
    expect(id.value).toBe('inst-b')
  })

  it('shares module-scoped state across callers', () => {
    const a = useActiveInstance()
    const b = useActiveInstance()
    a.set('inst-shared')
    expect(b.id.value).toBe('inst-shared')
  })
})
