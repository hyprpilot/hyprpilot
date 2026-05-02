import { beforeEach, describe, expect, it } from 'vitest'

import { __resetCwdHistoryForTests, pushCwd, useCwdHistory } from './use-cwd-history'

beforeEach(() => {
  __resetCwdHistoryForTests()
})

describe('useCwdHistory', () => {
  it('starts empty', () => {
    const { history } = useCwdHistory()

    expect(history.value).toEqual([])
  })

  it('pushCwd prepends new entries (MRU)', () => {
    const { history } = useCwdHistory()

    pushCwd('/tmp/a')
    pushCwd('/tmp/b')

    expect(history.value).toEqual(['/tmp/b', '/tmp/a'])
  })

  it('pushCwd deduplicates by exact match and re-promotes', () => {
    const { history } = useCwdHistory()

    pushCwd('/tmp/a')
    pushCwd('/tmp/b')
    pushCwd('/tmp/a')

    expect(history.value).toEqual(['/tmp/a', '/tmp/b'])
  })

  it('pushCwd ignores empty / whitespace input', () => {
    const { history } = useCwdHistory()

    pushCwd('')
    pushCwd('   ')

    expect(history.value).toEqual([])
  })

  it('pushCwd trims whitespace around the entry', () => {
    const { history } = useCwdHistory()

    pushCwd('  /tmp/a  ')

    expect(history.value).toEqual(['/tmp/a'])
  })

  it('caps history at MAX_HISTORY (10) entries — oldest drops first', () => {
    const { history } = useCwdHistory()

    for (let i = 0; i < 12; i++) {
      pushCwd(`/tmp/${i}`)
    }

    expect(history.value).toHaveLength(10)
    expect(history.value[0]).toBe('/tmp/11')
    expect(history.value[9]).toBe('/tmp/2')
  })

  it('persists history to localStorage', () => {
    pushCwd('/tmp/a')
    pushCwd('/tmp/b')

    const raw = window.localStorage.getItem('hyprpilot.cwdHistory')

    expect(raw).toBeTruthy()
    const parsed = JSON.parse(raw as string) as string[]

    expect(parsed).toEqual(['/tmp/b', '/tmp/a'])
  })

  it('clear() empties history and storage', () => {
    const { history, clear } = useCwdHistory()

    pushCwd('/tmp/a')
    clear()

    expect(history.value).toEqual([])
    expect(window.localStorage.getItem('hyprpilot.cwdHistory')).toBe('[]')
  })

  it('shares state across useCwdHistory calls', () => {
    const a = useCwdHistory()
    const b = useCwdHistory()

    a.push('/tmp/a')

    expect(b.history.value).toEqual(['/tmp/a'])
  })
})
