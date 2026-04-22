import { mount } from '@vue/test-utils'
import { describe, expect, it } from 'vitest'

import AgentToolCall from '@components/chat/messages/AgentToolCall.vue'
import { type ToolCallSnapshot } from '@composables'

function call(overrides: Partial<ToolCallSnapshot> = {}): ToolCallSnapshot {
  return {
    toolCallId: 'tc-1',
    title: 'read file',
    status: 'completed',
    content: [],
    ...overrides
  }
}

describe('AgentToolCall.vue', () => {
  it('escapes non-text block JSON before splicing into v-html', () => {
    const snapshot = call({
      content: [
        {
          type: 'image',
          payload: '<script>alert(1)</script>',
          marker: '<img src=x onerror=bad>'
        }
      ]
    })
    const wrapper = mount(AgentToolCall, { props: { call: snapshot } })

    const html = wrapper.html()
    expect(html).not.toContain('<script>alert(1)</script>')
    expect(html).not.toContain('<img src=x onerror=bad>')
    expect(html).toContain('&lt;script&gt;alert(1)&lt;/script&gt;')
    expect(html).toContain('&lt;img src=x onerror=bad&gt;')

    const injected = wrapper.element.querySelectorAll('script, img')
    expect(injected).toHaveLength(0)
  })

  it('renders string text blocks through markdown, not raw JSON', () => {
    const snapshot = call({
      content: [{ type: 'text', text: '**bold**' }]
    })
    const wrapper = mount(AgentToolCall, { props: { call: snapshot } })
    expect(wrapper.html()).toContain('<strong>bold</strong>')
  })

  it('shows the title and status in the header', () => {
    const wrapper = mount(AgentToolCall, { props: { call: call({ title: 'edit', status: 'pending' }) } })
    expect(wrapper.text()).toContain('edit')
    expect(wrapper.text()).toContain('pending')
  })
})
