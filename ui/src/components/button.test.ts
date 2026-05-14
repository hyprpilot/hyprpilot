import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import Button from './Button.vue'
import { ButtonTone, ButtonVariant } from '@components'

describe('Button.vue', () => {
  it('renders slot content', () => {
    const wrapper = mount(Button, { slots: { default: 'allow' } })

    expect(wrapper.text()).toBe('allow')
  })

  it('emits click', async() => {
    const wrapper = mount(Button)

    await wrapper.trigger('click')

    expect(wrapper.emitted('click')).toHaveLength(1)
  })

  it('applies variant and tone classes', () => {
    const wrapper = mount(Button, { props: { variant: ButtonVariant.Solid, tone: ButtonTone.Err } })

    expect(wrapper.classes()).toContain('is-solid')
    expect(wrapper.classes()).toContain('is-tone-err')
  })

  it('honors the disabled prop', () => {
    const wrapper = mount(Button, { props: { disabled: true } })

    expect(wrapper.attributes('disabled')).toBeDefined()
  })
})
