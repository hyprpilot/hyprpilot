import { faPlay } from '@fortawesome/free-solid-svg-icons'
import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import CommandPaletteRow from './CommandPaletteRow.vue'

describe('CommandPaletteRow.vue', () => {
  it('renders every slot', () => {
    const icon = faPlay
    const wrapper = mount(CommandPaletteRow, {
      props: {
        icon,
        label: 'submit',
        hint: 'enter',
        right: 'enter'
      }
    })

    expect(wrapper.find('.palette-row-icon svg').exists()).toBe(true)
    expect(wrapper.text()).toContain('submit')
    expect(wrapper.text()).toContain('enter')
  })

  it('reflects selected + danger on data attributes', () => {
    const wrapper = mount(CommandPaletteRow, {
      props: {
        selected: true,
        danger: true,
        label: 'nuke'
      }
    })

    expect(wrapper.attributes('data-selected')).toBe('true')
    expect(wrapper.attributes('data-danger')).toBe('true')
  })

  it('emits select / hover', async() => {
    const wrapper = mount(CommandPaletteRow, { props: { label: 'x' } })

    await wrapper.trigger('click')
    await wrapper.trigger('mouseenter')

    expect(wrapper.emitted('select')).toHaveLength(1)
    expect(wrapper.emitted('hover')).toHaveLength(1)
  })
})
