import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import Toast from './Toast.vue'
import { ToastTone } from './types'

describe('Toast.vue', () => {
  it('renders message + dismiss button by default', () => {
    const wrapper = mount(Toast, { props: { message: 'session resumed' } })

    expect(wrapper.text()).toContain('session resumed')
    expect(wrapper.find('button[aria-label="dismiss"]').exists()).toBe(true)
  })

  it('emits dismiss on close', async () => {
    const wrapper = mount(Toast, { props: { message: 'x' } })

    await wrapper.find('button[aria-label="dismiss"]').trigger('click')

    expect(wrapper.emitted('dismiss')).toHaveLength(1)
  })

  it('hides the dismiss button when dismissible=false', () => {
    const wrapper = mount(Toast, { props: { message: 'x', dismissible: false } })

    expect(wrapper.find('button[aria-label="dismiss"]').exists()).toBe(false)
  })

  it('renders a FontAwesome tone icon per tone', () => {
    const ok = mount(Toast, { props: { message: 'a', tone: ToastTone.Ok } })
    const warn = mount(Toast, { props: { message: 'a', tone: ToastTone.Warn } })
    const err = mount(Toast, { props: { message: 'a', tone: ToastTone.Err } })

    for (const w of [ok, warn, err]) {
      expect(w.find('.toast-tone-icon').exists()).toBe(true)
      expect(w.find('svg').exists()).toBe(true)
    }
  })

  it('uses role="alert" for errors and role="status" otherwise', () => {
    const ok = mount(Toast, { props: { message: 'a', tone: ToastTone.Ok } })
    const warn = mount(Toast, { props: { message: 'a', tone: ToastTone.Warn } })
    const err = mount(Toast, { props: { message: 'a', tone: ToastTone.Err } })

    expect(ok.find('.toast').attributes('role')).toBe('status')
    expect(warn.find('.toast').attributes('role')).toBe('status')
    expect(err.find('.toast').attributes('role')).toBe('alert')
  })
})
