import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import ToolPillStats from './ToolPillStats.vue'

describe('ToolPillStats.vue', () => {
  it('renders a duration pill formatted via formatDuration', () => {
    const w = mount(ToolPillStats, { props: { stats: [{ kind: 'duration', ms: 1500 }] } })

    expect(w.text()).toContain('1s')
    expect(w.findAll('.stat-pill')).toHaveLength(1)
  })

  it('splits diff into ok / err pills, hides zero sides', () => {
    const w = mount(ToolPillStats, {
      props: {
        stats: [{
          kind: 'diff', added: 12, removed: 3
        }]
      }
    })
    const pills = w.findAll('.stat-pill')

    expect(pills).toHaveLength(2)
    expect(pills[0].text()).toBe('+12')
    expect(pills[0].attributes('data-tone')).toBe('ok')
    expect(pills[1].text()).toBe('−3')
    expect(pills[1].attributes('data-tone')).toBe('err')
  })

  it('hides the diff entirely when both sides are zero', () => {
    const w = mount(ToolPillStats, {
      props: {
        stats: [{
          kind: 'diff', added: 0, removed: 0
        }]
      }
    })

    expect(w.findAll('.stat-pill')).toHaveLength(0)
  })

  it('renders text variant verbatim', () => {
    const w = mount(ToolPillStats, { props: { stats: [{ kind: 'text', value: '3 / 5' }] } })

    expect(w.text()).toBe('3 / 5')
  })

  it('lays out multiple stats in order', () => {
    const w = mount(ToolPillStats, {
      props: {
        stats: [
          { kind: 'duration', ms: 850 },
          {
            kind: 'diff', added: 20, removed: 0
          }
        ]
      }
    })
    const pills = w.findAll('.stat-pill')

    expect(pills).toHaveLength(2)
    expect(pills[0].text()).toBe('850ms')
    expect(pills[1].text()).toBe('+20')
  })

  it('renders nothing when stats array is empty', () => {
    const w = mount(ToolPillStats, { props: { stats: [] } })

    expect(w.find('.stat-pills').exists()).toBe(false)
  })
})
