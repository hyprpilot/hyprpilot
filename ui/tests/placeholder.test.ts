import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import Placeholder from '@views/Placeholder.vue'

describe('Placeholder.vue', () => {
  it('renders the product name and the shadcn-vue button', () => {
    const wrapper = mount(Placeholder)

    expect(wrapper.get('[data-testid="placeholder"]').text()).toContain('hyprpilot')
    expect(wrapper.get('[data-testid="placeholder-button"]').text()).toBe('Not implemented')
  })
})
