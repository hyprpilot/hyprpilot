import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import StatPill from './StatPill.vue'

describe('StatPill.vue', () => {
  it('renders the label', () => {
    const w = mount(StatPill, { props: { label: '2.4s' } })

    expect(w.text()).toBe('2.4s')
  })

  it('defaults to neutral tone, no pulsing dot', () => {
    const w = mount(StatPill, { props: { label: '2.4s' } })

    expect(w.attributes('data-tone')).toBe('neutral')
    expect(w.attributes('data-live')).toBe('false')
    expect(w.find('.stat-pill-dot').exists()).toBe(false)
  })

  it('reflects ok / err tones via data attribute', () => {
    expect(mount(StatPill, { props: { label: '+12', tone: 'ok' } }).attributes('data-tone')).toBe('ok')
    expect(mount(StatPill, { props: { label: '−3', tone: 'err' } }).attributes('data-tone')).toBe('err')
  })

  it('renders pulsating dot in live state', () => {
    const w = mount(StatPill, { props: { label: '13s', live: true } })

    expect(w.attributes('data-live')).toBe('true')
    expect(w.find('.stat-pill-dot').exists()).toBe(true)
  })
})
