import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import DesignShowcase from './DesignShowcase.vue'

describe('DesignShowcase.vue', () => {
  it('mounts all 9 default + 3 narrow stories without throwing', () => {
    const wrapper = mount(DesignShowcase)

    const sections = wrapper.findAll('.design-showcase-section')
    expect(sections).toHaveLength(12)
  })

  it('labels every story with its n + name', () => {
    const wrapper = mount(DesignShowcase)

    const labels = wrapper.findAll('.design-showcase-label-name').map((el) => el.text())
    expect(labels).toEqual([
      'Idle',
      'Conversation',
      'Tool calls',
      'Permission',
      'Queue',
      'Palette — root',
      'Palette — modes',
      'Palette — pickers',
      'Palette — sessions',
      'Idle · narrow',
      'Conversation · narrow',
      'Tool calls · narrow'
    ])
  })

  it('frames each story inside a .design-showcase-frame', () => {
    const wrapper = mount(DesignShowcase)

    const frame = wrapper.find('.design-showcase-frame')
    // scoped styles put width/height literals; testable via computed-style
    // isn't reliable in jsdom, so assert the class is present and the
    // structure wraps one child (the story component).
    expect(frame.exists()).toBe(true)
    expect(frame.element.children.length).toBe(1)
  })

  it('renders narrow-frame variants with inline width styles', () => {
    const wrapper = mount(DesignShowcase)

    const narrowFrames = wrapper.findAll('.design-showcase-frame-narrow')
    expect(narrowFrames).toHaveLength(3)
    for (const frame of narrowFrames) {
      expect((frame.element as HTMLElement).style.width).toBe('360px')
    }
  })
})
