import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import BreadcrumbPill from './BreadcrumbPill.vue'

describe('BreadcrumbPill.vue', () => {
  it('renders label + count', () => {
    const wrapper = mount(BreadcrumbPill, { props: { label: 'msgs', count: 3 } })

    expect(wrapper.text()).toContain('msgs')
    expect(wrapper.text()).toContain('3')
  })

  it('threads the accent color through the style var', () => {
    const wrapper = mount(BreadcrumbPill, {
      props: {
        color: '#abcdef',
        label: 'x',
        count: 1
      }
    })

    // jsdom may normalise hex to rgb(); accept either form.
    const style = wrapper.attributes('style') ?? ''

    expect(style).toMatch(/#abcdef|rgb\(\s*171,\s*205,\s*239\s*\)/)
  })
})
