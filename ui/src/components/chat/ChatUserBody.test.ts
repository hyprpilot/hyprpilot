import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import ChatUserBody from './ChatUserBody.vue'

describe('ChatUserBody.vue', () => {
  it('renders slot content', () => {
    const wrapper = mount(ChatUserBody, { slots: { default: 'hey there' } })

    expect(wrapper.text()).toBe('hey there')
  })
})
