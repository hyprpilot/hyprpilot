import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import ChatStreamCard from './ChatStreamCard.vue'
import { PlanStatus, StreamKind } from '../types'

describe('ChatStreamCard.vue', () => {
  it('active + items → checklist with per-status icons', () => {
    const wrapper = mount(ChatStreamCard, {
      props: {
        kind: StreamKind.Planning,
        active: true,
        label: 'planning',
        items: [
          { status: PlanStatus.Completed, text: 'read file' },
          { status: PlanStatus.InProgress, text: 'write file' },
          { status: PlanStatus.Pending, text: 'run tests' }
        ]
      }
    })

    const rows = wrapper.findAll('li')
    expect(rows).toHaveLength(3)
    expect(wrapper.text()).toContain('read file')
    expect(rows[0]!.attributes('data-status')).toBe('completed')
    expect(rows[1]!.attributes('data-status')).toBe('in_progress')
    expect(rows[2]!.attributes('data-status')).toBe('pending')
    expect(rows[0]!.find('svg').exists()).toBe(true)
    expect(rows[1]!.find('svg').exists()).toBe(true)
    expect(rows[2]!.find('svg').exists()).toBe(true)
  })

  it('active + slot → <pre> block', () => {
    const wrapper = mount(ChatStreamCard, {
      props: { kind: StreamKind.Thinking, active: true, label: 'thinking' },
      slots: { default: 'i should...' }
    })

    expect(wrapper.find('pre').exists()).toBe(true)
    expect(wrapper.find('pre').text()).toContain('i should...')
  })

  it('inactive → summary line', () => {
    const wrapper = mount(ChatStreamCard, {
      props: {
        kind: StreamKind.Thinking,
        active: false,
        label: 'thinking',
        summary: 'collected 18 clues'
      }
    })

    expect(wrapper.find('pre').exists()).toBe(false)
    expect(wrapper.find('ul').exists()).toBe(false)
    expect(wrapper.text()).toContain('collected 18 clues')
  })
})
