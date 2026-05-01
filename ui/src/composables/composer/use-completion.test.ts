import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { CompletionKind, TauriCommand } from '@ipc'

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }))

vi.mock('@ipc/bridge', async () => ({
  ...(await vi.importActual<object>('@ipc/bridge')),
  invoke: (cmd: string, args?: Record<string, unknown>) => invoke(cmd, args),
  listen: () => Promise.resolve(() => {})
}))

import { __resetUseCompletionForTests, useCompletion } from './use-completion'

const flushMicrotasks = (): Promise<void> => new Promise((r) => setTimeout(r, 0))
const wait = (ms: number): Promise<void> => new Promise((r) => setTimeout(r, ms))

describe('useCompletion', () => {
  beforeEach(() => {
    invoke.mockReset()
    __resetUseCompletionForTests()
  })

  afterEach(() => {
    __resetUseCompletionForTests()
  })

  it('opens the popover when daemon returns items', async () => {
    invoke.mockResolvedValueOnce({
      requestId: 'r1',
      sourceId: 'skills',
      replacementRange: { start: 0, end: 1 },
      items: [
        {
          label: 'git-commit',
          kind: CompletionKind.Skill,
          replacement: { range: { start: 0, end: 1 }, text: '#{skills://git-commit}' }
        }
      ]
    })
    const c = useCompletion()
    c.query('#g', 2)
    await wait(50)

    expect(invoke).toHaveBeenCalledWith(TauriCommand.CompletionQuery, expect.any(Object))
    expect(c.state.value.open).toBe(true)
    expect(c.state.value.items).toHaveLength(1)
    expect(c.state.value.sourceId).toBe('skills')
  })

  it('closes when daemon returns no items', async () => {
    invoke.mockResolvedValueOnce({
      requestId: 'r1',
      sourceId: null,
      replacementRange: null,
      items: []
    })
    const c = useCompletion()
    c.state.value.open = true
    c.query('hello', 5)
    await wait(50)

    expect(c.state.value.open).toBe(false)
  })

  it('cycles selectedIndex on selectNext / selectPrev', async () => {
    invoke.mockResolvedValueOnce({
      requestId: 'r1',
      sourceId: 'skills',
      replacementRange: { start: 0, end: 1 },
      items: [
        { label: 'a', kind: CompletionKind.Skill, replacement: { range: { start: 0, end: 1 }, text: 'a' } },
        { label: 'b', kind: CompletionKind.Skill, replacement: { range: { start: 0, end: 1 }, text: 'b' } },
        { label: 'c', kind: CompletionKind.Skill, replacement: { range: { start: 0, end: 1 }, text: 'c' } }
      ]
    })
    const c = useCompletion()
    c.query('#a', 2)
    await wait(50)
    expect(c.state.value.selectedIndex).toBe(0)
    c.selectNext()
    expect(c.state.value.selectedIndex).toBe(1)
    c.selectNext()
    c.selectNext()
    expect(c.state.value.selectedIndex).toBe(0) // wraps
    c.selectPrev()
    expect(c.state.value.selectedIndex).toBe(2) // wraps backward
  })

  it('commit returns the selected item and closes', async () => {
    invoke.mockResolvedValueOnce({
      requestId: 'r1',
      sourceId: 'skills',
      replacementRange: { start: 0, end: 2 },
      items: [
        {
          label: 'git-commit',
          kind: CompletionKind.Skill,
          replacement: { range: { start: 0, end: 2 }, text: '#{skills://git-commit}' }
        }
      ]
    })
    invoke.mockResolvedValueOnce({ cancelled: true }) // close → cancel
    const c = useCompletion()
    c.query('#g', 2)
    await wait(50)
    const item = c.commit()
    expect(item?.label).toBe('git-commit')
    expect(c.state.value.open).toBe(false)
  })

  it('close cancels the in-flight request via completion/cancel', async () => {
    invoke.mockResolvedValueOnce({
      requestId: 'r1',
      sourceId: 'skills',
      replacementRange: { start: 0, end: 1 },
      items: [
        { label: 'a', kind: CompletionKind.Skill, replacement: { range: { start: 0, end: 1 }, text: 'a' } }
      ]
    })
    invoke.mockResolvedValueOnce({ cancelled: true }) // for the cancel call
    const c = useCompletion()
    c.query('#a', 2)
    await wait(50)
    expect(c.state.value.latestQueryId).toBe('r1')
    c.close()
    await flushMicrotasks()
    expect(invoke).toHaveBeenCalledWith(TauriCommand.CompletionCancel, { requestId: 'r1' })
    expect(c.state.value.open).toBe(false)
  })
})
