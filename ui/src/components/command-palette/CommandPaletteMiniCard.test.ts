import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import CommandPaletteMiniCard from './CommandPaletteMiniCard.vue'

describe('CommandPaletteMiniCard.vue', () => {
  it('renders title + body slot', () => {
    const wrapper = mount(CommandPaletteMiniCard, {
      props: { title: 'recent' },
      slots: { default: '<div data-testid="body">rows</div>' }
    })

    expect(wrapper.text()).toContain('recent')
    expect(wrapper.find('[data-testid="body"]').exists()).toBe(true)
  })

  it('omits the title bar when not supplied', () => {
    const wrapper = mount(CommandPaletteMiniCard, { slots: { default: '<div />' } })

    expect(wrapper.find('.mini-palette-card-title').exists()).toBe(false)
  })
})
