import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import Composer from '@components/chat/Composer.vue'

describe('Composer.vue', () => {
  it('emits submit with the trimmed text when Enter is pressed without Shift', async () => {
    const wrapper = mount(Composer)
    const ta = wrapper.get('[data-testid="composer-textarea"]')
    await ta.setValue('  hi there  ')
    await ta.trigger('keydown', { key: 'Enter' })

    const emitted = wrapper.emitted('submit')
    expect(emitted).toBeDefined()
    expect(emitted![0]).toEqual(['hi there'])
  })

  it('inserts a newline when Shift-Enter is pressed (no submit)', async () => {
    const wrapper = mount(Composer)
    const ta = wrapper.get('[data-testid="composer-textarea"]')
    await ta.setValue('line 1')
    await ta.trigger('keydown', { key: 'Enter', shiftKey: true })

    expect(wrapper.emitted('submit')).toBeUndefined()
  })

  it('skips submit on empty or whitespace-only input', async () => {
    const wrapper = mount(Composer)
    const ta = wrapper.get('[data-testid="composer-textarea"]')
    await ta.setValue('   ')
    await ta.trigger('keydown', { key: 'Enter' })

    expect(wrapper.emitted('submit')).toBeUndefined()
  })

  it('form submit event also fires the submit emit', async () => {
    const wrapper = mount(Composer)
    const ta = wrapper.get('[data-testid="composer-textarea"]')
    await ta.setValue('click me')
    await wrapper.get('[data-testid="composer"]').trigger('submit')

    const emitted = wrapper.emitted('submit')
    expect(emitted).toBeDefined()
    expect(emitted![0]).toEqual(['click me'])
  })

  it('does not submit when sending is true (disabled gate)', async () => {
    const wrapper = mount(Composer, { props: { sending: true } })
    const ta = wrapper.get('[data-testid="composer-textarea"]')
    await ta.setValue('typing while sending')
    await ta.trigger('keydown', { key: 'Enter' })

    expect(wrapper.emitted('submit')).toBeUndefined()
  })

  it('skips submit while IME composition is active', async () => {
    const wrapper = mount(Composer)
    const ta = wrapper.get('[data-testid="composer-textarea"]')
    await ta.setValue('hello')
    await ta.trigger('keydown', { key: 'Enter', isComposing: true })

    expect(wrapper.emitted('submit')).toBeUndefined()
  })

  it('emits cancel when the cancel button is clicked while sending', async () => {
    const wrapper = mount(Composer, { props: { sending: true } })
    await wrapper.get('[data-testid="composer-cancel"]').trigger('click')
    expect(wrapper.emitted('cancel')).toBeDefined()
  })

  it('preserves the draft when sending settles without an explicit clear() call', async () => {
    const wrapper = mount(Composer, { props: { sending: true } })
    const ta = wrapper.get('[data-testid="composer-textarea"]')
    await ta.setValue('unsent draft')
    // parent reports failure — sending flips back to false without the
    // parent calling clear(); the old watcher would have wiped the draft.
    await wrapper.setProps({ sending: false } as Record<string, unknown>)
    expect((ta.element as HTMLTextAreaElement).value).toBe('unsent draft')
  })

  it('exposes clear() so the parent can wipe the draft after a successful submit', async () => {
    const wrapper = mount(Composer)
    const ta = wrapper.get('[data-testid="composer-textarea"]')
    await ta.setValue('will be wiped')
    ;(wrapper.vm as unknown as { clear: () => void }).clear()
    await wrapper.vm.$nextTick()
    expect((ta.element as HTMLTextAreaElement).value).toBe('')
  })
})
