import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import ChatRoleTag from './ChatRoleTag.vue'
import { Role } from '../types'

describe('ChatRoleTag.vue', () => {
  it('derives the glyph from the first letter of the label', () => {
    const wrapper = mount(ChatRoleTag, { props: { role: Role.User, label: 'captain' } })

    expect(wrapper.text()).toContain('C')
    expect(wrapper.text()).toContain('captain')
    expect(wrapper.attributes('data-role')).toBe('user')
  })

  it('uppercases the first glyph regardless of label casing', () => {
    const wrapper = mount(ChatRoleTag, { props: { role: Role.Assistant, label: 'pilot' } })

    expect(wrapper.text()).toContain('P')
    expect(wrapper.text()).toContain('pilot')
    expect(wrapper.attributes('data-role')).toBe('assistant')
  })

  it('honours custom labels', () => {
    const wrapper = mount(ChatRoleTag, { props: { role: Role.User, label: 'operator' } })

    expect(wrapper.text()).toContain('O')
    expect(wrapper.text()).toContain('operator')
  })
})
