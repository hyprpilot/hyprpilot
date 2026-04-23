import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import CommandPaletteShell from './CommandPaletteShell.vue'

describe('CommandPaletteShell.vue', () => {
  it('renders every slot', () => {
    const wrapper = mount(CommandPaletteShell, {
      slots: {
        title: '<span data-testid="t">title</span>',
        query: '<input data-testid="q" />',
        body: '<ul data-testid="b"><li>a</li></ul>',
        hints: '<kbd data-testid="h">k</kbd>'
      }
    })

    expect(wrapper.find('[data-testid="t"]').exists()).toBe(true)
    expect(wrapper.find('[data-testid="q"]').exists()).toBe(true)
    expect(wrapper.find('[data-testid="b"]').exists()).toBe(true)
    expect(wrapper.find('[data-testid="h"]').exists()).toBe(true)
  })

  it('defaults to width="default"', () => {
    const wrapper = mount(CommandPaletteShell, { slots: { body: '<ul />' } })

    expect(wrapper.attributes('data-width')).toBe('default')
  })

  it('auto-promotes to wide when the preview slot is bound', () => {
    const wrapper = mount(CommandPaletteShell, {
      slots: {
        body: '<ul />',
        preview: '<section data-testid="p" />'
      }
    })

    expect(wrapper.attributes('data-width')).toBe('wide')
    expect(wrapper.find('[data-testid="p"]').exists()).toBe(true)
  })
})
