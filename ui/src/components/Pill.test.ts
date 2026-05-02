import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import Pill from './Pill.vue'

describe('Pill.vue', () => {
  it('renders slot content', () => {
    const wrapper = mount(Pill, { slots: { default: 'hello' } })

    expect(wrapper.text()).toBe('hello')
  })

  it('applies custom color to border-color style', () => {
    const wrapper = mount(Pill, { props: { color: '#abcdef' }, slots: { default: 'x' } })

    // jsdom normalises hex to rgb() — assert both the property and the
    // equivalent rgb triple so the test is stable across jsdom versions.
    const style = wrapper.attributes('style') ?? ''

    expect(style).toContain('border-color')
    expect(style).toMatch(/#abcdef|rgb\(\s*171,\s*205,\s*239\s*\)/)
  })

  it('toggles the mono class when mono=true', () => {
    const wrapper = mount(Pill, { props: { mono: true }, slots: { default: 'x' } })

    expect(wrapper.classes()).toContain('is-mono')
  })
})
