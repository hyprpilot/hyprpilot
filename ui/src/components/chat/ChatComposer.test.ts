import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import ChatComposer from './ChatComposer.vue'
import type { ComposerPill } from '../types'

describe('ChatComposer.vue', () => {
  it('renders pills + removes them', async () => {
    const pills: ComposerPill[] = [
      { id: 'a', label: 'file://src/App.vue', kind: 'attachment' },
      { id: 'b', label: 'skills/debug', kind: 'skill' }
    ]
    const wrapper = mount(ChatComposer, { props: { pills } })

    expect(wrapper.findAll('.composer-pill')).toHaveLength(2)
    await wrapper.findAll('button[aria-label="remove"]')[0]!.trigger('click')
    expect(wrapper.emitted('removePill')?.[0]).toEqual(['a'])
  })

  it('disables submit for empty or sending state', async () => {
    const wrapper = mount(ChatComposer, { props: { sending: true } })
    const submit = wrapper.get('[data-testid="composer-submit"]')

    expect(submit.attributes('disabled')).toBeDefined()
    expect(submit.attributes('aria-label')).toBe('sending')
  })

  it('emits submit with trimmed text', async () => {
    const wrapper = mount(ChatComposer)
    const textarea = wrapper.get<HTMLTextAreaElement>('[data-testid="composer-textarea"]')

    await textarea.setValue('  hello  ')
    await wrapper.trigger('submit')

    expect(wrapper.emitted('submit')?.[0]).toEqual(['hello'])
  })

  it('enter submits, shift+enter does not', async () => {
    const wrapper = mount(ChatComposer)
    const textarea = wrapper.get<HTMLTextAreaElement>('[data-testid="composer-textarea"]')
    await textarea.setValue('hi')

    await textarea.trigger('keydown', { key: 'Enter', shiftKey: true })
    expect(wrapper.emitted('submit')).toBeUndefined()

    await textarea.trigger('keydown', { key: 'Enter' })
    expect(wrapper.emitted('submit')?.[0]).toEqual(['hi'])
  })
})
