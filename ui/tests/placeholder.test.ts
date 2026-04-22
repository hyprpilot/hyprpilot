import { mount } from '@vue/test-utils'
import { describe, expect, it, vi } from 'vitest'

import Placeholder from '@views/Placeholder.vue'

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn()
}))

vi.mock('@tauri-apps/api/event', () => ({
  listen: vi.fn(() => Promise.resolve(() => {}))
}))

describe('Placeholder.vue', () => {
  it('renders the product name and the submit control', () => {
    const wrapper = mount(Placeholder)

    expect(wrapper.get('[data-testid="placeholder"]').text()).toContain('hyprpilot')
    expect(wrapper.find('[data-testid="placeholder-submit"]').exists()).toBe(true)
    expect(wrapper.find('[data-testid="placeholder-textarea"]').exists()).toBe(true)
  })
})
