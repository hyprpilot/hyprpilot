import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'

import { __resetUseCompletionForTests, useCompletion } from './use-completion'
import { CompletionKind, TauriCommand } from '@ipc'

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }))

vi.mock('@ipc', async() => ({
  ...(await vi.importActual<object>('@ipc')),
  invoke: (cmd: string, args?: Record<string, unknown>) => invoke(cmd, args),
  listen: () => Promise.resolve(() => {})
}))

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

  it('opens the popover when daemon returns items', async() => {
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

  it('closes when daemon returns no items', async() => {
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

  it('cycles selectedIndex on selectNext / selectPrev', async() => {
    invoke.mockResolvedValueOnce({
      requestId: 'r1',
      sourceId: 'skills',
      replacementRange: { start: 0, end: 1 },
      items: [
        {
          label: 'a',
          kind: CompletionKind.Skill,
          replacement: { range: { start: 0, end: 1 }, text: 'a' }
        },
        {
          label: 'b',
          kind: CompletionKind.Skill,
          replacement: { range: { start: 0, end: 1 }, text: 'b' }
        },
        {
          label: 'c',
          kind: CompletionKind.Skill,
          replacement: { range: { start: 0, end: 1 }, text: 'c' }
        }
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

  it('commit returns the selected item and closes', async() => {
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

  it('close cancels the in-flight request via completion/cancel', async() => {
    invoke.mockResolvedValueOnce({
      requestId: 'r1',
      sourceId: 'skills',
      replacementRange: { start: 0, end: 1 },
      items: [
        {
          label: 'a',
          kind: CompletionKind.Skill,
          replacement: { range: { start: 0, end: 1 }, text: 'a' }
        }
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

  /**
   * Pin the contract: when `close()` runs between an in-flight
   * `completion/query` issue and its response (e.g. captain hits
   * Enter to send while ripgrep is mid-walk), the response handler
   * MUST NOT reopen the popover. Without the generation guard, the
   * stale response races past the daemon's best-effort cancel and
   * re-shows the previous completion items after the buffer was
   * just submitted.
   */
  it('drops in-flight response when close() runs between issue and resolution', async() => {
    let resolveQuery: (value: unknown) => void = () => {}

    invoke.mockReturnValueOnce(
      new Promise((resolve) => {
        resolveQuery = resolve
      })
    )
    const c = useCompletion()

    c.query('#git', 4, { manual: true })
    // Manual debounce is 0; advance past setTimeout(fn, 0).
    await flushMicrotasks()
    // Query is now mid-await. Close before the response lands.
    c.close()
    expect(c.state.value.open).toBe(false)

    // Daemon was already past the cancel point, response races back.
    resolveQuery({
      requestId: 'r-stale',
      sourceId: 'skills',
      replacementRange: { start: 0, end: 4 },
      items: [
        {
          label: 'git-commit',
          kind: CompletionKind.Skill,
          replacement: { range: { start: 0, end: 4 }, text: '#{git-commit}' }
        }
      ]
    })
    await flushMicrotasks()
    await flushMicrotasks()

    // Popover stays closed; stale items don't appear.
    expect(c.state.value.open).toBe(false)
    expect(c.state.value.items).toHaveLength(0)
  })
})
