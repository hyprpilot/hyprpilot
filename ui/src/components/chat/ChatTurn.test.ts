import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import ChatTurn from './ChatTurn.vue'
import { Role } from '../types'

describe('ChatTurn.vue', () => {
  it('renders the role tag + default slot', () => {
    const wrapper = mount(ChatTurn, {
      props: { role: Role.User, elapsed: '2s' },
      slots: { default: '<p>hi</p>' }
    })

    expect(wrapper.attributes('data-role')).toBe('user')
    expect(wrapper.text()).toContain('captain')
    expect(wrapper.text()).toContain('hi')
    expect(wrapper.text()).toContain('2s')
  })

  it('renders elapsed with a pulse dot when live', () => {
    const wrapper = mount(ChatTurn, { props: { role: Role.Assistant, live: true, elapsed: '9s' } })

    expect(wrapper.attributes('data-live')).toBe('true')
    expect(wrapper.text()).toContain('9s')
    expect(wrapper.find('[aria-hidden="true"]').exists()).toBe(true)
  })
})
