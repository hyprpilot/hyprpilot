import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import ChatAssistantBody from './ChatAssistantBody.vue'

describe('ChatAssistantBody.vue', () => {
  it('renders slot content', () => {
    const wrapper = mount(ChatAssistantBody, { slots: { default: 'done.' } })

    expect(wrapper.text()).toBe('done.')
  })
})
