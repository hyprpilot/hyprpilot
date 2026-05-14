import { faXmark } from '@fortawesome/free-solid-svg-icons'
import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import KbdHint from './KbdHint.vue'

describe('KbdHint.vue', () => {
  it('renders every string key + the label', () => {
    const wrapper = mount(KbdHint, { props: { keys: ['Ctrl', 'K'], label: 'palette' } })

    const keys = wrapper.findAll('kbd')

    expect(keys).toHaveLength(2)
    expect(keys[0]!.text()).toBe('Ctrl')
    expect(keys[1]!.text()).toBe('K')
    expect(wrapper.text()).toContain('palette')
  })

  it('renders FontAwesome glyphs for IconDefinition entries and text for strings', () => {
    const wrapper = mount(KbdHint, {
      props: { keys: ['Ctrl', faXmark], label: 'close' }
    })

    const keys = wrapper.findAll('kbd')

    expect(keys).toHaveLength(2)
    // First keycap — plain text.
    expect(keys[0]!.text()).toBe('Ctrl')
    expect(keys[0]!.find('svg').exists()).toBe(false)
    // Second keycap — FontAwesome svg, no text body.
    expect(keys[1]!.find('svg').exists()).toBe(true)
    expect(keys[1]!.text()).toBe('')
  })
})
